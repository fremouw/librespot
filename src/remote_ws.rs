use std;
use log::{debug, info};
use librespot::playback::player::{PlayerEvent, PlayerEventChannel};
use std::sync::Arc;
use std::{thread, time};
use librespot::connect::spirc::Spirc;
use futures::{Async, Future, Poll, Stream};
use std::sync::mpsc::channel;
use websocket::client::ClientBuilder;
use websocket::{Message, OwnedMessage};
use url::Url;
use serde_json::json;
use serde_json::{Value, Map};

#[derive(Clone, Debug)]
pub struct RemoteWsConfig {
    pub uri: String,
    pub volume: u16,
    pub input_source: Option<String>,
}

pub struct RemoteWs {
    internal_thread_handle: Option<thread::JoinHandle<()>>,
}

struct RemoteWsInternal {
    config: RemoteWsConfig,
    spirc: Arc<Spirc>,
    event_channel: PlayerEventChannel,
    rpc_next_id: i64,
    ws_tx: Option<std::sync::mpsc::Sender<websocket::OwnedMessage>>,
    tx_thread_handle: Option<thread::JoinHandle<()>>,
    rx_thread_handle: Option<thread::JoinHandle<()>>,
}

impl RemoteWs {
    pub fn new(
        config: RemoteWsConfig,
        spirc: Arc<Spirc>,
        event_channel: PlayerEventChannel,
    ) -> RemoteWs {
        let _handle = thread::spawn(move || { 
            debug!("starting new RemoteWsThread");

            let internal = RemoteWsInternal {
                config: config,
                spirc: spirc,
                event_channel: event_channel,
                rpc_next_id: 1,
                ws_tx: None,
                tx_thread_handle: None,
                rx_thread_handle: None,
            };

            let _ = internal.wait();
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
        loop {
            let ref mut event_channel_ = self.event_channel;
            match event_channel_.poll() {
                Ok(Async::Ready(Some(event))) => {
                    self.handle_event(event);
                },
                Ok(Async::Ready(None)) => return Ok(Async::Ready(())),
                Ok(Async::NotReady) => {
                    return Ok(Async::NotReady);
                },
                Err(e) => {
                    return Err(From::from(e));
                },
            };

            thread::sleep(time::Duration::from_millis(1));
        }
    }
}

impl RemoteWsInternal {
    pub fn disconnect(&mut self) {
        debug!("disconnect");

        if let Some(ref mut ws_tx) = self.ws_tx {
            match ws_tx.send(OwnedMessage::Close(None)) {
                Ok(_) => (),
                Err(_) => debug!("error sending close message"),
            };
        }

        self.ws_tx = None;

        if let Some(handle) = self.tx_thread_handle.take() {
            match handle.join() {
                Ok(_) => debug!("closed send thread"),
                Err(_) => debug!("error closing send thread"),
            };
        } else {
            debug!("unable to exit send thread");
        }
        
        self.tx_thread_handle = None;

        if let Some(handle) = self.rx_thread_handle.take() {
            match handle.join() {
                Ok(_) => debug!("closed receive thread"),
                Err(_) => debug!("error closing receive thread"),
            };
        } else {
            debug!("unable to exit receive thread");
        }

        self.rx_thread_handle = None;
    }

    fn connect(&mut self) {
        let uri = Url::parse(&self.config.uri).unwrap();

        let _client = match ClientBuilder::new(&uri.to_string()).unwrap().connect_insecure() {
            Ok(c) => c,
            Err(e) => {
                panic!("can't connect to {}, error {:?}", uri.to_string(), e);
            }
        };

        info!("successfully connected");

        let (mut receiver, mut sender) = _client.split().unwrap();
    
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
                        debug!("close message");
                        let _ = sender.send_message(&message);
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
        let _input_source = self.config.input_source.clone();
        let _rx_handle = thread::spawn(move || {
            let mut current_input_source: String = "".to_string();

            if let Some(ref __input_source) = _input_source {
                current_input_source = __input_source.clone();
            }

            // Receive loop
            for message in receiver.incoming_messages() {
                let message = match message {
                    Ok(m) => m,
                    Err(e) => {
                        debug!("error receiving message: {:?}", e);
                        let _ = tx_1.send(OwnedMessage::Close(None));
                        return;
                    }
                };
                match message {
                    OwnedMessage::Close(_) => {
                        debug!("close message");
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

                            _spirc.volume_set(new_volume as u16);
                        }
                        else if v["method"] == "inputSourceChanged" {
                            let source = v["params"].as_str().unwrap();
                            
                            current_input_source = source.to_string().clone();

                            if let Some(ref __input_source) = _input_source {
                                if source != __input_source {
                                    _spirc.pause();
                                } 
                            }
                        }
                        else if v["method"] == "powerStateChanged" {
                            let state = v["params"].as_str().unwrap();

                            if state == "Standby" || state == "Off" {
                                _spirc.pause();
                            }
                        }
                        else if v["method"] == "muteStateChanged" {
                            let _mute = v["params"].as_bool().unwrap();

                            if let Some(ref __input_source) = _input_source {
                                if current_input_source.as_str() == __input_source {
                                    if _mute {
                                        _spirc.pause();
                                    } else {
                                        _spirc.play();
                                    }
                                }
                            }
                        }
                        else {
                            debug!("msg: {}", text);
                        }
                    }
                    OwnedMessage::Ping(data) => {
                        match tx_1.send(OwnedMessage::Pong(data)) {
                            // Send a pong in response
                            Ok(()) => (),
                            Err(e) => {
                                debug!("error receiving ping message: {:?}", e);
                                let _ = tx_1.send(OwnedMessage::Close(None));
                                return;
                            }
                        }
                    }
                    // Say what we received
                    _ => debug!("Receive Loopx: {:?}", message),
                }
            }
        });

        self.rx_thread_handle = Some(_rx_handle);

        let converted_volume = f64::from(self.config.volume) / f64::from(u16::max_value()) * 100.0;
        
        let mut param = Map::new();

        param.insert("state".to_string(), json!("On".to_string()));
        param.insert("volume".to_string(), json!(converted_volume.round() as u16));
        param.insert("muted".to_string(), json!(false));

        if let Some(ref _input_source) = self.config.input_source {
            param.insert("inputSource".to_string(), json!(_input_source));
        }

        self.send_command("setContext".to_string(), Value::Object(param));
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
                let converted_volume = f64::from(volume) / f64::from(u16::max_value()) * 100.0;

                let param = json!(converted_volume.round() as u32);
                self.send_command("setVolume".to_string(), param);
            }
            _ => return,
        }
    }

    fn send_command(&mut self, method: String, params: serde_json::Value) {
        let id = self.rpc_next_id;
        self.rpc_next_id += 1;

        let _cmd = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        debug!("JSON: {}", _cmd.to_string());
        if let Some(ref mut ws_tx) = self.ws_tx {
            match ws_tx.send(OwnedMessage::Text(_cmd.to_string())) {
                Ok(()) => (),
                Err(e) => {
                    debug!("ERROR sending message: {:?}", e);

                    self.disconnect();

                    self.connect();
                }
            };
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

        self.disconnect();
    }
}
