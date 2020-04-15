use std;
// use librespot::core::session::Session;
use librespot::playback::player::{PlayerEvent, PlayerEventChannel};

use std::sync::Arc;
use std::thread;
use librespot::connect::spirc::Spirc;

use futures::{Async, Future, Poll, Stream};

use websocket::client::ClientBuilder;
use websocket::{Message, OwnedMessage};
use url::Url;
use serde_json::json;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct RemoteWsConfig {
    pub uri: String,
}

pub struct RemoteWs {
}

struct RemoteWsInternal {
    config: RemoteWsConfig,
    spirc: Arc<Spirc>,
    event_channel: PlayerEventChannel,
    rpc_last_id: i64,
    ws_tx: Option<websocket::sender::Writer<std::net::TcpStream>>,
    rx_thread_handle: Option<thread::JoinHandle<()>>,
}

impl RemoteWs {
    pub fn new(
        config: RemoteWsConfig,
        spirc: Arc<Spirc>,
        event_channel: PlayerEventChannel,
    ) -> RemoteWs {
        let _handle = thread::spawn(move || { 
            println!("Starting new RemoteWsThread[]");

            let internal = RemoteWsInternal {
                config: config,
                spirc: spirc,
                event_channel: event_channel,
                rpc_last_id: 1,
                ws_tx: None,
                rx_thread_handle: None,
            };

            let _ = internal.wait();
            println!("Starting new RemoteWsThread[] finished");
        });
    
        RemoteWs {
        }
    }
}

impl Future for RemoteWsInternal {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<(), ()> {
        println!("RemoteWsInternal::poll");

        println!("RemoteWsInternal::poll:loop");
        loop {
            let ref mut event_channel_ = self.event_channel;
            if let Async::Ready(Some(event)) = event_channel_.poll().unwrap() {
                self.handle_event(event);
            } 
        }
    }
}

impl RemoteWsInternal {
    fn disconnect(&mut self) {

        if let Some(ref mut ws_tx) = self.ws_tx {
            ws_tx.send_message(&Message::close()).unwrap();
        }

        if let Some(handle) = self.rx_thread_handle.take() {
            match handle.join() {
                Ok(_) => println!("Closed MetaPipe thread"),
                Err(_) => println!("MetaPipe panicked!"),
            }
        } else {
            println!("Unable to exit RemoteWs");
        }

        self.rx_thread_handle = None;
        self.ws_tx = None;

        println!("disconnect RemoteWs");
    }

    fn connect(&mut self) {
        println!("RemoteWsInternal::connect");

        let uri = Url::parse(&self.config.uri).unwrap();

        let client = ClientBuilder::new(&uri.to_string())
            .unwrap()
            .add_protocol("rust-websocket")
            .connect_insecure()
            .unwrap();

	    println!("Successfully connected");

        let (mut receiver, sender) = client.split().unwrap();
        
        self.ws_tx = Some(sender);

        let _spirc = self.spirc.clone();
        let _rx_loop = thread::spawn(move || {
            // Receive loop
            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Receive Loop: {:?}", e);
                        // let _ = tx_1.send(OwnedMessage::Close(None));
                        return;
                    }
                };
                match message {
                    OwnedMessage::Close(_) => {
                        println!("CLOSE");
                        // Got a close message, so send a close message and return
                        // let _ = tx_1.send(OwnedMessage::Close(None));
                        return;
                    }
                    OwnedMessage::Text(text) => {
                        let v: Value = serde_json::from_str(&text).unwrap();

                        if v["method"] == "volumeChanged" {
                            let volume: f64 = v["params"].as_f64().unwrap();
                            let new_volume = (volume * f64::from(u16::max_value())) / 100.0;

                            println!("new volume {}, conv: {}", volume, new_volume);

                            // let ref spirc = self.spirc;
                            // spirc.volume_set(new_volume as u16);
                            _spirc.volume_set(new_volume as u16);
                        }
                    }
                    // OwnedMessage::Ping(data) => {
                    //     match tx_1.send(OwnedMessage::Pong(data)) {
                    //         // Send a pong in response
                    //         Ok(()) => (),
                    //         Err(e) => {
                    //             println!("Receive Loop: {:?}", e);
                    //             return;
                    //         }
                    //     }
                    // }
                    // Say what we received
                    _ => println!("Receive Loop: {:?}", message),
                }
            }
        });

        self.rx_thread_handle = Some(_rx_loop);

        println!("RemoteWsInternal::connect::end");
    }
    
    fn handle_event(&mut self, event: PlayerEvent) {
        println!("RemoteWsInternal::handle_event");

        match event {
            PlayerEvent::Changed {
                old_track_id,
                new_track_id,
            } => {
                println!("Changed {:?} to {:?}", old_track_id, new_track_id);
            }
            PlayerEvent::Started { track_id, .. } => {
                println!("Started {:?}", track_id);

                self.connect();
            }
            PlayerEvent::Stopped { track_id, .. } => {
                println!("Stopped {:?}", track_id);

                self.disconnect();
            }
            PlayerEvent::VolumeSet { volume, .. } => {
                let mixer_volume = f64::from(volume) / f64::from(u16::max_value()) * 100.0;
        
                println!("set_volume: {} converted {}", volume, mixer_volume);
        
                let id = self.rpc_last_id;
                self.rpc_last_id += 1;

                let _cmd = json!({
                    "id": id,
                    "method": "setVolume",
                    "params": mixer_volume.round(),
                });
        
                if let Some(ref mut ws_tx) = self.ws_tx {
                    ws_tx.send_message(&OwnedMessage::Text(_cmd.to_string())).unwrap();
                }
            }
            _ => return,
        }
    }
}
