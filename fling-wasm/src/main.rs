use anyhow::{anyhow, Context, Result};
use futures::channel::mpsc::{self, UnboundedSender};
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use iroh_blobs::ticket::BlobTicket;
use shared::{
    app_events::AppEvent, network::Network, ui_events::UIEvent,
    websocket_messages::WebSocketMessage,
};
use ui::UI;
use wasm_bindgen_futures::spawn_local;
use web_sys::{
    console::{self},
    Blob, Window,
};

use crate::iroh_node::IrohNode;

mod iroh_node;

const WS_URL: &str = "wss://fling-server.fly.dev/ws";

struct WasmNetwork {
    to_ws: mpsc::UnboundedSender<UIEvent>,
    tx: mpsc::UnboundedSender<AppEvent>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
}

impl WasmNetwork {
    fn new(nickname: &str) -> Self {
        let (to_ws, mut from_ui) = mpsc::unbounded::<UIEvent>();
        let (tx, rx) = mpsc::unbounded::<AppEvent>();

        let mut tx_clone = tx.clone();
        spawn_local(async move {
            let ws_init = async move {
                let ws = WebSocket::open(WS_URL).context("can't connect to ws")?;
                let (write, read) = ws.split();
                Ok::<_, anyhow::Error>((write, read))
            };
            let iroh_init = async move {
                let iroh_node = IrohNode::new().await?;
                Ok(iroh_node)
            };

            let tx_clone_1 = tx_clone.clone();
            let result = futures::try_join!(ws_init, iroh_init);
            match result {
                Ok(((mut write, mut read), iroh_node)) => {
                    spawn_local(async move {
                        while let Some(msg) = read.next().await {
                            match msg {
                                Ok(m) => process_message(m, tx_clone_1.clone()).await,
                                Err(e) => {}
                            }
                        }
                    });

                    spawn_local(async move {
                        while let Some(msg) = from_ui.next().await {
                            match msg {
                                UIEvent::PrepareFile { recipient, files } => {
                                    let blobs = files
                                        .into_iter()
                                        .map(|f| {
                                            (Blob::from(f.inner().to_owned()), f.inner().name())
                                        })
                                        .collect::<Vec<_>>();

                                    let tt = iroh_node.import(blobs).await.unwrap();

                                    let ticket = BlobTicket::new(
                                        iroh_node.endpoint.addr(),
                                        tt.hash(),
                                        iroh_blobs::BlobFormat::HashSeq,
                                    )
                                    .to_string();

                                    let json =
                                        WebSocketMessage::SendFile { recipient, ticket }.to_json();

                                    write.send(Message::Text(json.into())).await.unwrap();
                                    console::log_1(&"sent".into());
                                }
                                _ => {
                                    let json = msg.to_ws().expect("shouldn't happen");
                                    let json = json.to_json();
                                    write.send(Message::Text(json)).await.ok();
                                }
                            }
                        }
                    });

                    tx_clone.send(AppEvent::ReadyToPublishUser).await.ok();
                }
                Err(e) => {}
            }
        });

        Self { to_ws, tx, rx }
    }
}

impl Network for WasmNetwork {
    fn send(&self, event: shared::app_events::AppEvent) {
        self.tx.unbounded_send(event).ok();
    }

    fn send_ws(&self, ws_msg: shared::ui_events::UIEvent) -> Result<()> {
        self.to_ws
            .unbounded_send(ws_msg)
            .map_err(|e| anyhow!(e.to_string()).context("websocket send failed"))
    }

    fn try_recv(&mut self) -> Option<shared::app_events::AppEvent> {
        self.rx.try_next().ok().flatten()
    }

    fn open_file_dialog(&mut self) {
        let mut tx = self.tx.clone();
        spawn_local(async move {
            use rfd::AsyncFileDialog;

            let files = AsyncFileDialog::new().pick_files().await;
            if let Some(file_handles) = files {
                tx.send(AppEvent::ReceivedFile(file_handles)).await.ok();
            }
        });
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {
    use eframe::wasm_bindgen::JsCast as _;

    eframe::WebLogger::init(log::LevelFilter::Debug).ok();

    let web_options = eframe::WebOptions::default();

    spawn_local(async {
        use std::path::PathBuf;

        let window = web_sys::window().expect("no window");
        let document = window.document().expect("no document");

        let canvas = document
            .get_element_by_id("the_canvas_id")
            .expect("failed to get canvas id")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("canvas id isn't an HtmlCanvasElement");

        let nickname = get_nickname(&window);
        let wasm_network = WasmNetwork::new(&nickname);

        let start_result = eframe::WebRunner::new()
            .start(
                canvas,
                web_options,
                Box::new(|cc| {
                    Ok(Box::new(UI::new(
                        cc,
                        nickname,
                        PathBuf::new(),
                        wasm_network,
                    )))
                }),
            )
            .await;

        if let Some(loading_text) = document.get_element_by_id("loading_text") {
            match start_result {
                Ok(_) => loading_text.remove(),
                Err(e) => {
                    panic!("app crashed {e:?}");
                }
            }
        }
    });
}

fn get_nickname(window: &Window) -> String {
    let mut arr = [0u8; 3];
    let crypto = window.crypto().expect("no crypto");
    crypto.get_random_values_with_u8_array(&mut arr).unwrap();

    let adjectives = ["happy", "lucky", "brave", "calm", "bright"];
    let nouns = ["cat", "dog", "fox", "bear", "wolf"];

    let adj = adjectives[arr[0] as usize % adjectives.len()];
    let noun = nouns[arr[1] as usize % nouns.len()];
    let num = (arr[2] as usize % 1000) as u8;
    let name = format!("{}-{}-{}", adj, noun, num);

    name
}

async fn process_message(message: Message, mut tx: UnboundedSender<AppEvent>) {
    if let Message::Text(s) = message {
        match serde_json::from_str::<WebSocketMessage>(&s) {
            Ok(msg) => match msg {
                WebSocketMessage::RegisterSuccess(users) => {
                    tx.send(AppEvent::RegisterSuccess(users)).await.ok();
                }
                _ => {}
            },
            Err(e) => {}
        }
    }
}
