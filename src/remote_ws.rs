use std;
use log::{debug, error, info};
use librespot::core::session::Session;
use librespot::playback::player::{PlayerEvent, PlayerEventChannel};
use std::sync::Arc;
// use std::thread;
use std::{thread, time};
use librespot::connect::spirc::Spirc;

use futures::{Async, Future, Poll, Stream};

use std::sync::mpsc::channel;
use websocket::client::ClientBuilder;
use websocket::client::sync::Client;
use websocket::{Message, OwnedMessage};
use url::Url;
use serde_json::json;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct RemoteWsConfig {
    pub uri: String,
}

pub struct RemoteWs {
    internal_thread_handle: Option<thread::JoinHandle<()>>,
}

struct RemoteWsInternal {
    config: RemoteWsConfig,
    spirc: Arc<Spirc>,
    event_channel: PlayerEventChannel,
    rpc_next_id: i64,
    ws_reconnect_count: i32,
    ws_tx: Option<std::sync::mpsc::Sender<websocket::OwnedMessage>>,
    tx_thread_handle: Option<thread::JoinHandle<()>>,
    rx_thread_handle: Option<thread::JoinHandle<()>>,
    // ws_client: Option<websocket::client::sync::Client<Autostream>>,
}

impl RemoteWs {
    pub fn new(
        config: RemoteWsConfig,
        spirc: Arc<Spirc>,
        event_channel: PlayerEventChannel,
    ) -> RemoteWs {
        let _handle = thread::spawn(move || { 
            debug!("Starting new RemoteWsThread[]");

            let internal = RemoteWsInternal {
                config: config,
                spirc: spirc,
                event_channel: event_channel,
                rpc_next_id: 1,
                ws_reconnect_count: 0,
                ws_tx: None,
                tx_thread_handle: None,
                rx_thread_handle: None,
            };

            let _ = internal.wait();
            debug!("Starting new RemoteWsThread[] finished");
        });
    
        RemoteWs {
            internal_thread_handle: Some(_handle),
        }
    }
}

impl Future for RemoteWsInternal {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        debug!("RemoteWsInternal::poll");

        loop {
            let ref mut event_channel_ = self.event_channel;
            match event_channel_.poll() {
                Ok(Async::Ready(Some(event))) => {
                    self.handle_event(event);
                },
                Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                Ok(Async::NotReady) => (),
                Err(_) => debug!("errrrorrrr"),
            };

            thread::sleep(time::Duration::from_millis(1));
        }
    }
}

impl RemoteWsInternal {
    fn disconnect(&mut self) {
        debug!("disconnect RemoteWs");

        // if let Some(ref mut ws_tx) = self.ws_tx {
        //     match ws_tx.send(&Message::close()) {
        //         Ok(()) => (),
        //         Err(_) => debug!("Already closed?"),
        //     };
        // }

        self.ws_tx = None;

        if let Some(handle) = self.rx_thread_handle.take() {
            match handle.join() {
                Ok(_) => debug!("Closed RemoteWs thread"),
                Err(_) => debug!("RemoteWs panicked!"),
            };
        } else {
            debug!("Unable to exit RemoteWs");
        }

        self.rx_thread_handle = None;

        debug!("disconnect RemoteWs end");
    }

    fn connect(&mut self) {
        debug!("RemoteWsInternal::connect");

        let uri = Url::parse(&self.config.uri).unwrap();

        let client = ClientBuilder::new(&uri.to_string())
            .unwrap()
            .add_protocol("rust-websocket")
            .connect_insecure()
            .unwrap();

        info!("Successfully connected");
        
        self.ws_reconnect_count = 0;

        let (mut receiver, mut sender) = client.split().unwrap();
        let (tx, rx) = channel();

        let tx_1 = tx.clone();

        self.ws_tx = Some(tx);

        let _tx_handle = thread::spawn(move || {
            loop {
                let message = match rx.recv() {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Send Loop: {:?}", e);
                        return;
                    }
                };
                match message {
                    OwnedMessage::Close(_) => {
                        //let _ = sender.send_message(&message);
                        // If it's a close message, just send it and then return.
                        return;
                    }
                    _ => (),
                }
                // Send the message
                match sender.send_message(&message) {
                    Ok(()) => (),
                    Err(e) => {
                        println!("Send Loop: {:?}", e);
                        let _ = sender.send_message(&Message::close());
                        return;
                    }
                }
            }
        });

        self.tx_thread_handle = Some(_tx_handle);

        let _spirc = self.spirc.clone();
        let _rx_handle = thread::spawn(move || {
            // Receive loop
            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        debug!("Receive Loop: {:?}", e);
                        
                        let _ = tx_1.send(OwnedMessage::Close(None));
                        return;
                    }
                };
                match message {
                    OwnedMessage::Close(_) => {
                        debug!("CLOSE");
                        // Got a close message, so send a close message and return
                        let _ = tx_1.send(OwnedMessage::Close(None));
                        return;
                    }
                    OwnedMessage::Text(text) => {
                        let v: Value = serde_json::from_str(&text).unwrap();

                        if v["method"] == "volumeChanged" {
                            let volume: f64 = v["params"].as_f64().unwrap();
                            let new_volume = (volume * f64::from(u16::max_value())) / 100.0;

                            debug!("new volume {}, conv: {}", volume, new_volume);

                            // let ref spirc = self.spirc;
                            // spirc.volume_set(new_volume as u16);
                            _spirc.volume_set(new_volume as u16);
                        }
                    }
                    OwnedMessage::Ping(data) => {
                        match tx_1.send(OwnedMessage::Pong(data)) {
                            // Send a pong in response
                            Ok(()) => (),
                            Err(e) => {
                                println!("Receive Loopp: {:?}", e);
                                return;
                            }
                        }
                    }
                    // Say what we received
                    _ => debug!("Receive Loopx: {:?}", message),
                }
            }

            debug!("exit rx loop");
        });

        self.rx_thread_handle = Some(_rx_handle);

        debug!("RemoteWsInternal::connect::end");
    }
    
    fn handle_event(&mut self, event: PlayerEvent) {
        debug!("RemoteWsInternal::handle_event");

        match event {
            PlayerEvent::Changed {
                old_track_id,
                new_track_id,
            } => {
                debug!("Changed {:?} to {:?}", old_track_id, new_track_id);
            }
            PlayerEvent::Started { track_id, .. } => {
                debug!("Started {:?}", track_id);

                self.connect();
            }
            PlayerEvent::Stopped { track_id, .. } => {
                debug!("Stopped {:?}", track_id);

                self.disconnect();
            }
            PlayerEvent::VolumeSet { volume, .. } => {
                let mixer_volume = f64::from(volume) / f64::from(u16::max_value()) * 100.0;
        
                debug!("set_volume: {} converted {}", volume, mixer_volume);
        
                let id = self.rpc_next_id;
                self.rpc_next_id += 1;

                let _cmd = json!({
                    "id": id,
                    "method": "setVolume",
                    "params": mixer_volume.round(),
                });
        
                if let Some(ref mut ws_tx) = self.ws_tx {
                    // ws_tx.send(OwnedMessage::Text(_cmd.to_string()));
                    match ws_tx.send(OwnedMessage::Text(_cmd.to_string())) {
                        Ok(()) => (),
						Err(e) => {
                            debug!("ERROR sending message: {:?}", e);

                            self.disconnect();
                            // self.connect();
						}
                    };
                }
            }
            _ => return,
        }
    }
}

impl Drop for RemoteWs {
    fn drop(&mut self) {
        debug!("drop RemoteWs");

        if let Some(handle) = self.internal_thread_handle.take() {
            match handle.join() {
                Ok(_) => debug!("Closed RemoteWs internal thread"),
                Err(_) => debug!("RemoteWs internal panicked!"),
            };
        }
    }
}

impl Drop for RemoteWsInternal {
    fn drop(&mut self) {
        debug!("drop RemoteWsInternal");

        drop(self.ws_tx.take());

       // drop(self.ws_rx.take());

        // if let Some(handle) = self.rx_thread_handle.take() {
        //     match handle.join() {
        //         Ok(_) => debug!("Closed RemoteWs rx thread"),
        //         Err(_) => debug!("RemoteWs rx panicked!"),
        //     };
        // }

        // if let Some(handle) = self.tx_thread_handle.take() {
        //     match handle.join() {
        //         Ok(_) => debug!("Closed RemoteWs tx thread"),
        //         Err(_) => debug!("RemoteWs tx panicked!"),
        //     };
        // }

        debug!("drop RemoteWsInternal dropped");
        // drop(self.task_tx.take());
        // if let Some(handle) = self.thread_handle.take() {
        //     match handle.join() {
        //         Ok(_) => debug!("Closed RemoteWs thread"),
        //         Err(_) => error!("RemoteWs panicked!"),
        //     }
        // } else {
        //     warn!("Unable to drop RemoteWs");
        // }
    }
}
