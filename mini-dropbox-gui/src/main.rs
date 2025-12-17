use std::{ops::ControlFlow, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Result};
use directories::ProjectDirs;
use eframe::CreationContext;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::oneshot::{self, Receiver, Sender};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Bytes, Message},
};

mod modals;
mod requests;
mod responses;

#[tokio::main]
async fn main() -> eframe::Result {
    // env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_resizable(true),
        ..Default::default()
    };

    eframe::run_native(
        "shit app",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

#[derive(Serialize, Deserialize, Default)]
struct PersistedState {
    username: String,
}

#[derive(Default)]
pub struct MyApp {
    path: String,
    reqwest_client: Arc<Client>,
    show_popup_username: bool,
    username: String,
    username_rx: Option<Receiver<Result<String>>>,
    create_username_message: String,

    should_open_username_modal: bool,
}

impl MyApp {
    fn new(cc: &CreationContext) -> Self {
        let config_path = get_config_path();
        let state = load_state(config_path);

        Self {
            reqwest_client: Arc::new(reqwest::Client::new()),
            show_popup_username: true,
            username_rx: None,
            should_open_username_modal: true,
            username: state.username,
            ..Default::default()
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &eframe::egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ctx.set_pixels_per_point(2.0);

            self.poll_username_response();

            self.show_username_modal(&ui);
            if !self.path.is_empty() {
                ui.label(self.path.clone());
            }
            ui.label(format!("Username is {}", self.username));
            if ui.button("Test API").clicked() {
                tokio::spawn(async move {
                    let ws_stream = match connect_async("ws://127.0.0.1:3000/ws").await {
                        Ok((stream, response)) => {
                            println!("Server response was {response:?}");
                            stream
                        }
                        Err(e) => {
                            println!("Websocket handshake failed for client: {e}");
                            return;
                        }
                    };

                    let (mut sender, mut receiver) = ws_stream.split();

                    tokio::spawn(async move {
                        while let Some(Ok(msg)) = receiver.next().await {
                            if process_message(msg).is_break() {
                                break;
                            }
                        }
                    });
                });
            }
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(e) = save_state(self.username.clone()) {
            println!("Err saving state: {e}");
            todo!("Display a modal");
        }
    }
}

fn process_message(msg: Message) -> ControlFlow<(), ()> {
    match msg {
        Message::Ping(b) => {
            println!("Got ping {:?}", b)
        }
        _ => {}
    }
    ControlFlow::Continue(())
}

fn get_config_path() -> PathBuf {
    let proj_dirs =
        ProjectDirs::from("com", "lambs", "mini-dropbox").expect("cannot find project dir");
    proj_dirs.config_dir().join("config.json")
}

fn load_state(path: PathBuf) -> PersistedState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PersistedState>(&s).ok())
        .unwrap_or_default()
}

fn save_state(username: String) -> Result<()> {
    // if fails, defaults to empty username
    let persisted_state_json =
        serde_json::to_string(&PersistedState { username }).unwrap_or_default();

    let config_path = get_config_path();
    std::fs::create_dir_all(
        config_path
            .parent()
            .ok_or_else(|| anyhow!("Error creating config dir"))?,
    )
    .ok();

    std::fs::write(config_path, persisted_state_json)?;

    Ok(())
}
