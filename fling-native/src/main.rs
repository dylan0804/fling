use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use futures_util::{SinkExt, StreamExt};
use iroh::protocol::Router;
use iroh_blobs::{
    api::remote::GetProgressItem, format::collection::Collection,
    get::request::get_hash_seq_and_sizes, ticket::BlobTicket, BlobFormat,
};
use names::{Generator, Name};
use serde_json::{self};
use shared::{app_events::AppEvent, network::Network, websocket_messages::WebSocketMessage};
use tokio::sync::mpsc::{self};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use ui::UI;

use crate::iroh_node::IrohNode;

mod iroh_node;

const WS_URL: &str = "wss://fling-server.fly.dev/ws";

macro_rules! try_or_continue {
    ($result:expr, $tx:expr, $msg:expr) => {
        match $result {
            Ok(val) => val,
            Err(e) => {
                $tx.send(AppEvent::FatalError(anyhow!(e).context($msg)))
                    .ok();
                continue;
            }
        }
    };
}

pub struct NativeNetwork {
    tx: mpsc::UnboundedSender<AppEvent>,
    rx: mpsc::UnboundedReceiver<AppEvent>,
    to_ws: mpsc::UnboundedSender<WebSocketMessage>,
}

impl NativeNetwork {
    fn new(nickname: String, download_dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();
        let (to_ws, mut from_ui) = mpsc::unbounded_channel::<WebSocketMessage>();

        let tx_clone = tx.clone();
        let tx_clone_1 = tx.clone();
        tokio::spawn(async move {
            let ws_init = async {
                let ws_stream = connect_async(WS_URL)
                    .await
                    .context("WebSocket connection failed")?;

                let (sender, receiver) = ws_stream.0.split();
                Ok::<_, anyhow::Error>((sender, receiver))
            };

            let tx_clone = tx_clone.clone();
            let download_dir_1 = download_dir.clone();
            let iroh_init = async {
                let iroh_node = IrohNode::new(download_dir_1, nickname)
                    .await
                    .context("Iroh node initialization failed")?;

                let router = Router::builder(iroh_node.endpoint.clone())
                    .accept(iroh_blobs::ALPN, iroh_node.blobs.clone())
                    .spawn();

                Ok::<_, anyhow::Error>((iroh_node, router))
            };

            tokio::spawn(async move {
                match tokio::try_join!(ws_init, iroh_init) {
                    Ok(((mut sender, mut receiver), (iroh_node, router))) => {
                        // get ws msg
                        let tx_clone_1 = tx_clone.clone();
                        tokio::spawn(async move {
                            loop {
                                match receiver.next().await {
                                    Some(Ok(msg)) => process_message(msg, tx_clone_1.clone()).await,
                                    Some(Err(e)) => {
                                        tx_clone_1
                                            .send(AppEvent::FatalError(
                                                anyhow!(e).context("WebSocket error"),
                                            ))
                                            .ok();
                                        break;
                                    }
                                    None => {
                                        tx_clone_1
                                            .send(AppEvent::FatalError(anyhow!(
                                                "WebSocket disconnected"
                                            )))
                                            .ok();
                                        break;
                                    }
                                }
                            }
                        });

                        // send ws msg
                        let tx_clone = tx_clone.clone();
                        let download_dir = download_dir.clone();
                        tokio::spawn(async move {
                            while let Some(websocket_msg) = from_ui.recv().await {
                                match websocket_msg {
                                    WebSocketMessage::PrepareFile { recipient, files } => {
                                        tx_clone.send(AppEvent::ImportStart).ok();
                                        let tt = try_or_continue!(
                                            iroh_node.import(files, tx_clone.clone()).await,
                                            tx_clone,
                                            "Failed to import file(s)"
                                        );
                                        tx_clone.send(AppEvent::ImportDone).ok();
                                        let ticket = BlobTicket::new(
                                            iroh_node.endpoint.addr(),
                                            tt.hash(),
                                            BlobFormat::HashSeq,
                                        );

                                        let json = WebSocketMessage::SendFile { recipient, ticket }
                                            .to_json();

                                        if let Err(e) = sender
                                            .send(Message::Text(json.into()))
                                            .await
                                            .context("Websocket send failed")
                                        {
                                            tx_clone.send(AppEvent::FatalError(e)).ok();
                                        }
                                    }
                                    WebSocketMessage::DownloadFile(ticket) => {
                                        let downloader =
                                            iroh_node.store.downloader(&iroh_node.endpoint);
                                        match downloader
                                            .download(ticket.hash(), Some(ticket.addr().id))
                                            .await
                                        {
                                            Ok(_) => {
                                                let download_dir = download_dir.clone();
                                                let local_info = try_or_continue!(
                                                    iroh_node
                                                        .store
                                                        .remote()
                                                        .local(ticket.hash_and_format())
                                                        .await,
                                                    tx_clone,
                                                    "Failed to get local info"
                                                );

                                                if !local_info.is_complete() {
                                                    let connection = try_or_continue!(
                                                        iroh_node
                                                            .endpoint
                                                            .connect(
                                                                ticket.addr().id,
                                                                iroh_blobs::ALPN
                                                            )
                                                            .await,
                                                        tx_clone,
                                                        "Failed to connect to sender"
                                                    );
                                                    let (_, size) = try_or_continue!(
                                                        get_hash_seq_and_sizes(
                                                            &connection,
                                                            &ticket.hash(),
                                                            1024 * 1024 * 1024 * 5,
                                                            None
                                                        )
                                                        .await,
                                                        tx_clone,
                                                        "Failed to get file(s) size"
                                                    );

                                                    // skip the 1st index, bcs its the collection blob and obv we don't need it
                                                    let actual_size =
                                                        size.iter().skip(1).sum::<u64>();
                                                    let get = iroh_node.store.remote().execute_get(
                                                        connection,
                                                        local_info.missing(),
                                                    );
                                                    let mut stream = get.stream();
                                                    tx_clone.send(AppEvent::DownloadStart).ok();
                                                    while let Some(item) = stream.next().await {
                                                        match item {
                                                            GetProgressItem::Progress(b) => {
                                                                let value =
                                                                    b as f32 / actual_size as f32;
                                                                tx_clone.send(AppEvent::UpdateProgressValue(value)).ok();
                                                            }
                                                            GetProgressItem::Done(_) => {
                                                                tx_clone
                                                                    .send(AppEvent::DownloadDone)
                                                                    .ok();
                                                                break;
                                                            }
                                                            GetProgressItem::Error(e) => {
                                                                tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Error downloading one of the files"))).ok();
                                                            }
                                                        }
                                                    }
                                                }

                                                match Collection::load(
                                                    ticket.hash(),
                                                    iroh_node.store.as_ref(),
                                                )
                                                .await
                                                {
                                                    Ok(c) => {
                                                        for (f, h) in c.into_iter() {
                                                            let p = download_dir.join(f);
                                                            if let Err(e) = iroh_node
                                                                .store
                                                                .blobs()
                                                                .export(h, p)
                                                                .await
                                                            {
                                                                tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Error downloading file {p}: {e:?}")))
                                                                    .ok();
                                                            }
                                                        }
                                                        iroh_node
                                                            .store
                                                            .tags()
                                                            .delete_all()
                                                            .await
                                                            .unwrap();
                                                    }
                                                    Err(e) => {
                                                        tx_clone
                                                            .send(AppEvent::FatalError(e.context(
                                                                "Error loading collection",
                                                            )))
                                                            .ok();
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                tx_clone
                                                    .send(AppEvent::FatalError(
                                                        e.context("Download failed"),
                                                    ))
                                                    .ok();
                                            }
                                        }

                                        let _ = &router;
                                    }
                                    _ => {
                                        let json = websocket_msg.to_json();
                                        if let Err(e) = sender
                                            .send(Message::Text(json.into()))
                                            .await
                                            .context("Websocket send failed")
                                        {
                                            tx_clone.send(AppEvent::FatalError(e)).ok();
                                        }
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        let context = format!("Join handle error {:?}", e);
                        tx_clone.send(AppEvent::FatalError(e.context(context))).ok();
                    }
                }

                tx_clone_1.send(AppEvent::ReadyToPublishUser).ok();
            });
        });

        Self { tx, rx, to_ws }
    }
}

impl Network for NativeNetwork {
    fn send(&self, event: AppEvent) -> Result<()> {
        self.tx.send(event).context("receiver is closed")
    }

    fn send_ws(&self, ws_msg: WebSocketMessage) -> Result<()> {
        self.to_ws.send(ws_msg).context("ws receiver is closed")
    }

    fn try_recv(&mut self) -> Option<AppEvent> {
        self.rx.try_recv().ok()
    }

    fn clone_tx(&self) -> mpsc::UnboundedSender<AppEvent> {
        self.tx.clone()
    }
}

#[tokio::main]
async fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_resizable(true)
            .with_active(true)
            .with_inner_size([400.0, 500.0]),
        ..Default::default()
    };

    let mut generator = Generator::with_naming(Name::Numbered);
    let nickname = generator.next().unwrap_or("Guest".into());
    let download_dir = dirs::download_dir().unwrap_or_else(|| PathBuf::from("."));

    let native_network = NativeNetwork::new(nickname.clone(), download_dir.clone());

    eframe::run_native(
        "Fling",
        native_options,
        Box::new(|cc| {
            Ok(Box::new(UI::new(
                cc,
                nickname,
                download_dir,
                native_network,
            )))
        }),
    )
}

async fn process_message(msg: Message, tx: mpsc::UnboundedSender<AppEvent>) {
    if let Message::Text(bytes) = msg {
        match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
            Ok(websocket_msg) => match websocket_msg {
                WebSocketMessage::RegisterSuccess(current_users) => {
                    tx.send(AppEvent::RegisterSuccess(current_users)).ok();
                }
                WebSocketMessage::UserJoined(nickname) => {
                    tx.send(AppEvent::AddNewUser(nickname)).ok();
                }
                WebSocketMessage::UserLeft(nickname) => {
                    tx.send(AppEvent::RemoveUser(nickname)).ok();
                }
                WebSocketMessage::ReceiveFile(ticket) => {
                    tx.send(AppEvent::DownloadFile(ticket)).ok();
                }
                WebSocketMessage::ErrorDeserializingJson(e) => {
                    tx.send(AppEvent::FatalError(
                        anyhow!(e).context("Server JSON error"),
                    ))
                    .ok();
                }
                _ => {}
            },
            Err(e) => {
                tx.send(AppEvent::FatalError(
                    anyhow!(e).context("Message parse failed"),
                ))
                .ok();
            }
        }
    }
}
