use std;
use librespot::core::session::Session;
use librespot::playback::player::PlayerEventChannel;
use librespot::playback::player::PlayerEvent;

use std::io::ErrorKind;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use librespot::connect::spirc::Spirc;

use websocket::client::ClientBuilder;
use websocket::{Message, OwnedMessage};
use serde_json::json;
use serde_json::{Value};
use librespot_core::util;

#[derive(Clone, Debug)]
pub struct RemoteWsConfig {
    pub uri: String,
}

pub struct RemoteWs {
    pub thread_handle: Option<thread::JoinHandle<()>>,
    ws_tx: websocket::sender::Writer<std::net::TcpStream>,
}

struct RemoteWsThread {
    config: RemoteWsConfig,
    ws_rx: websocket::receiver::Reader<std::net::TcpStream>,
    spirc: Arc<Spirc>,
}

impl RemoteWs {
    pub fn new(
        config: RemoteWsConfig,
        spirc: Arc<Spirc>,
    ) -> (RemoteWs) {
        println!("REMOTEWS!!");

        println!("Connecting to {:?}", config.uri);

        let _client = ClientBuilder::new(&config.uri)
            .unwrap()
            .add_protocol("rust-websocket")
            .connect_insecure()
            .unwrap();

        let (mut receiver, mut sender) = _client.split().unwrap();

        println!("Successfully connected");

        let handle = thread::spawn(move || {
            println!("Starting new RemoteWsThread[]");

            let remote_ws_thread = RemoteWsThread {
                config: config,
                ws_rx: receiver,
                spirc: spirc,
            };

            remote_ws_thread.run();
        });
    
        (RemoteWs {
            thread_handle: Some(handle),
            ws_tx: sender,
        })
    }

    pub fn handle_event(&mut self, event: PlayerEvent) {
        println!("handle_event");
        match event {
            PlayerEvent::Changed {
                old_track_id,
                new_track_id,
            } => {
                println!("Changed");
            }
            PlayerEvent::Started { track_id, .. } => {
                println!("Started");
            }
            PlayerEvent::Stopped { track_id, .. } => {
                println!("Stopped");
            }
            PlayerEvent::VolumeSet { volume, .. } => {
                let mixer_volume = f64::from(volume) / f64::from(u16::max_value()) * 100.0;
        
                println!("set_volume: {} converted {}", volume, mixer_volume);
        
                let cmd = json!({
                    "id": 1,
                    "method": "setVolume",
                    "params": mixer_volume.round(),
                });
        
                let m = OwnedMessage::Text(cmd.to_string());
                self.ws_tx.send_message(&m);
            }
            _ => return,
        }
    }
}

impl RemoteWsThread {
    fn run(mut self) {
        for message in self.ws_rx.incoming_messages() {
            let message = match message {
                Ok(m) => m,
                Err(e) => {
                    println!("Receive Loop: {:?}", e);
                    return;
                }
            };
            match message {
                OwnedMessage::Close(_) => {
                    println!("AVR Close");
                    // Got a close message, so send a close message and return
                    // let _ = tx_1.send(OwnedMessage::Close(None));
                    return;
                }
                OwnedMessage::Text(text) => {
                    println!("txt msg: {:?}", text);

                    let v: Value = serde_json::from_str(&text).unwrap();

                    // Access parts of the data by indexing with square brackets.
                    println!("method {} at the vol {}", v["method"], v["params"]);
                    if(v["method"] == "volumeChanged") {
                        let mut volume: f64 = v["params"].as_f64().unwrap();
                        let new_volume = (volume * f64::from(u16::max_value())) / 100.0;

                        println!("new volume {}, conv: {}", volume, new_volume);
                        self.spirc.volume_set(new_volume as u16);
                        // let mixer_volume = f64::from(volume) / f64::from(u16::max_value()) * 100.0;

                        // self.spirc.volume_up();
                    }

                    // let c: futures::sync::mpsc::UnboundedSender<PlayerCommand> = cmd_rx;
                    // cmd_tx.unbounded_send(PlayerCommand::EmitVolumeSetEvent(20));
                    //.command(PlayerCommand::EmitVolumeSetEvent(20));
                    //commands.as_ref().unwrap().unbounded_send(PlayerCommand::EmitVolumeSetEvent(20)).unwrap();
                }
                
                // Say what we received
                _ => {
                    println!("Receive Loop: {:?}", message);
                },
            }
        }
    }
}
