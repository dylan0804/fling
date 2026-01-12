use std::{path::PathBuf, str::FromStr};

use anyhow::{anyhow, Context};
use eframe::CreationContext;
use egui::{Align2, Color32, CornerRadius, DroppedFile, Id, LayerId, RichText, Stroke, Vec2};
use egui_toast::{ToastKind, Toasts};
use futures_util::{SinkExt, StreamExt};
use iroh::protocol::Router;
use iroh_blobs::ticket::BlobTicket;
use names::{Generator, Name};
use rfd::FileDialog;
use serde_json::{self};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{events::AppEvent, iroh_node::IrohNode, message::WebSocketMessage, state::AppState};

mod events;
mod iroh_node;
mod message;
mod state;
mod toast;

const WS_URL: &'static str = "wss://rough-waterfall-3088.fly.dev/ws";

#[tokio::main]
async fn main() -> eframe::Result {
    // env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_resizable(true)
            .with_inner_size([400.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Mini Dropbox",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

pub struct MyApp {
    app_state: AppState,
    nickname: String,
    active_users_list: Vec<String>,
    toasts: Toasts,
    files: Vec<PathBuf>,
    dropped_files: Vec<DroppedFile>,
    selected_file: PathBuf,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    to_ws: Sender<WebSocketMessage>,
    download_dir: PathBuf,
}

impl MyApp {
    fn new(cc: &CreationContext) -> Self {
        egui_material_icons::initialize(&cc.egui_ctx);

        let (tx, rx) = mpsc::channel::<AppEvent>(100);
        let (to_ws, from_ui) = mpsc::channel::<WebSocketMessage>(100);

        let toasts = Toasts::new()
            .anchor(Align2::RIGHT_TOP, (-10., 10.))
            .order(egui::Order::Tooltip);

        Self {
            app_state: AppState::OnStartup(Some(from_ui)),
            files: vec![],
            dropped_files: vec![],
            selected_file: PathBuf::new(),
            active_users_list: vec![],
            nickname: String::new(),
            download_dir: PathBuf::new(),
            to_ws,
            toasts,
            rx,
            tx,
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.inactive.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.hovered.corner_radius = CornerRadius::same(8);
        style.visuals.widgets.active.corner_radius = CornerRadius::same(8);
        ctx.set_style(style);

        let accent_color = Color32::from_rgb(79, 140, 255);
        let bg_dark = Color32::from_rgb(30, 30, 30);
        let bg_card = Color32::from_rgb(40, 40, 40);
        let text_dim = Color32::from_rgb(140, 140, 140);

        // top panel
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(bg_dark).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui_material_icons::icon_text(egui_material_icons::icons::ICON_CIRCLE)
                            .color(Color32::from_rgb(80, 200, 120))
                            .size(12.0),
                    );
                    ui.label(RichText::new(&self.nickname).strong().size(14.0));
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg_dark).inner_margin(16.0))
            .show(ctx, |ui| {
                // event handler
                while let Ok(app_event) = self.rx.try_recv() {
                    match app_event {
                        AppEvent::ReadyToPublishUser => self.app_state = AppState::PublishUser,
                        AppEvent::RegisterSuccess => {
                            self.show_toast("Connected!", ToastKind::Success);
                            self.app_state = AppState::Ready;
                        }
                        AppEvent::UpdateActiveUsersList(active_users_list) => {
                            self.active_users_list = active_users_list;
                        }
                        AppEvent::FatalError(e) => {
                            self.show_toast(format!("{e:#}"), ToastKind::Error);
                        }
                        AppEvent::DownloadFile { ticket, file_name} => {
                            self.to_ws.try_send(WebSocketMessage::DownloadFile { ticket, file_name }).ok();
                        }
                    }
                }

                match &mut self.app_state {
                    AppState::OnStartup(from_ui) => {
                        let mut ws_receiver = from_ui.take().expect("from_ui already taken");

                        self.download_dir = dirs::download_dir().unwrap_or_else(|| {
                            self.tx.try_send(AppEvent::FatalError(anyhow!("Failed getting download dir, using \".\" instead"))).ok();
                            PathBuf::from(".")
                        });

                        let mut generator = Generator::with_naming(Name::Numbered);
                        self.nickname = generator.next().unwrap_or("Guest".into());

                        let tx = self.tx.clone();
                        let download_dir = self.download_dir.clone();
                        tokio::spawn(async move {
                            let ws_init = async {
                                let ws_stream = connect_async(WS_URL)
                                    .await
                                    .context("WebSocket connection failed")?;

                                let (sender, receiver) = ws_stream.0.split();
                                Ok::<_, anyhow::Error>((sender, receiver))
                            };

                            let iroh_init = async {
                                let iroh_node = IrohNode::new()
                                    .await
                                    .context("Iroh node initialization failed")?;

                                let router = Router::builder(iroh_node.endpoint.clone())
                                    .accept(iroh_blobs::ALPN, iroh_node.blobs.clone())
                                    .spawn();

                                Ok((iroh_node, router))
                            };

                            let download_dir = download_dir.clone(); 
                            tokio::spawn(async move {
                                match tokio::try_join!(ws_init, iroh_init) {
                                    Ok(((mut sender, mut receiver), (iroh_node, router))) => {
                                        // get ws msg
                                        let tx_clone = tx.clone();
                                        tokio::spawn(async move {
                                            loop {
                                                match receiver.next().await {
                                                    Some(Ok(msg)) => process_message(msg, tx_clone.clone()).await,
                                                    Some(Err(e)) => {
                                                        tx_clone.send(AppEvent::FatalError(anyhow!(e).context("WebSocket error"))).await.ok();
                                                        break;
                                                    }
                                                    None => {
                                                        tx_clone.send(AppEvent::FatalError(anyhow!("WebSocket disconnected"))).await.ok();
                                                        break;
                                                    }
                                                }
                                            }
                                        });

                                        // send ws msg
                                        let tx_clone = tx.clone();
                                        let download_dir = download_dir.clone();
                                        tokio::spawn(async move {
                                            while let Some(websocket_msg) = ws_receiver.recv().await {
                                                match websocket_msg {
                                                    WebSocketMessage::PrepareFile {
                                                        recipient,
                                                        abs_path,
                                                        file_name
                                                    } => {
                                                        let tag = match iroh_node
                                                            .store
                                                            .blobs()
                                                            .add_path(abs_path)
                                                            .await {
                                                                Ok(a) => a,
                                                                Err(e) => {
                                                                    tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Error getting TagInfo"))).await.ok();
                                                                    continue;
                                                                }
                                                        };

                                                        let node_id = iroh_node.endpoint.addr();

                                                        let ticket = BlobTicket::new(
                                                            node_id,
                                                            tag.hash,
                                                            tag.format,
                                                        )
                                                        .to_string();

                                                        let json = WebSocketMessage::SendFile { recipient, ticket, file_name }.to_json();

                                                        if let Err(e) = sender
                                                            .send(Message::Text(json.into()))
                                                            .await
                                                            .context("Websocket send failed")
                                                        {
                                                            tx_clone
                                                                .send(AppEvent::FatalError(e))
                                                                .await
                                                                .ok();
                                                        }
                                                    }
                                                    WebSocketMessage::DownloadFile { ticket, file_name } => {
                                                        let downloader = iroh_node.store.downloader(&iroh_node.endpoint);
                                                        match downloader.download(ticket.hash(), Some(ticket.addr().id)).await {
                                                            Ok(_) =>  {
                                                                let mut download_dir = download_dir.clone();
                                                                download_dir = download_dir.join(file_name);

                                                                if let Err(e) = iroh_node.store.blobs().export(ticket.hash(), download_dir).await {
                                                                   tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Failed to save file to Downloads"))).await.ok();
                                                                }
                                                            }
                                                            Err(e ) => {
                                                                tx_clone.send(AppEvent::FatalError(e.context("Download failed"))).await.ok();
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
                                                            tx_clone
                                                                .send(AppEvent::FatalError(e))
                                                                .await
                                                                .ok();
                                                        }
                                                    }
                                                }
                                            }
                                        });

                                        tx.send(AppEvent::ReadyToPublishUser).await.ok();
                                    }
                                    Err(e) => {
                                        let context = format!("Join handle error {:?}", e);
                                        tx.send(AppEvent::FatalError(e.context(context))).await.ok();
                                    }
                                }
                            });
                        });

                        self.app_state = AppState::Connecting;
                    }
                    AppState::Connecting => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            ui.add(egui::Spinner::new().size(32.0).color(accent_color));
                            ui.add_space(12.0);
                            ui.label(RichText::new("Connecting...").color(text_dim).size(14.0));
                        });
                    }
                    AppState::PublishUser => {
                        if let Err(e) = self.to_ws.try_send(WebSocketMessage::Register {
                            nickname: self.nickname.clone(),
                        }) {
                            self.tx
                                .try_send(AppEvent::FatalError(
                                    anyhow!(e).context("Register send failed"),
                                ))
                                .ok();
                        }
                        self.app_state = AppState::WaitForRegisterConfirmation;
                    }
                    AppState::WaitForRegisterConfirmation => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            ui.add(egui::Spinner::new().size(32.0).color(accent_color));
                            ui.add_space(12.0);
                            ui.label(RichText::new("Registering...").color(text_dim).size(14.0));
                        });
                    }
                    AppState::Ready => {
                        // file drop zone
                        egui::Frame::new()
                            .fill(bg_card)
                            .corner_radius(12.0)
                            .stroke(Stroke::new(2.0, Color32::from_rgb(60, 60, 60)))
                            .inner_margin(24.0)
                            .show(ui, |ui| {
                                ui.set_min_height(120.0);
                                ui.vertical_centered(|ui| {
                                    if self.files.is_empty() {
                                        ui.add_space(20.0);
                                        ui.label(RichText::new("📁").size(32.0));
                                        ui.add_space(8.0);
                                        if ui.link(RichText::new("Click to select a file").color(accent_color).size(14.0)).clicked() {
                                            if let Some(file) = FileDialog::new().set_directory("/").pick_file() {
                                                self.files.push(file);
                                            }
                                        }
                                        ui.label(RichText::new("or drag and drop").color(text_dim).size(12.0));
                                    } else {
                                        for file in &self.files {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("✓").color(Color32::from_rgb(80, 200, 120)));
                                                ui.label(RichText::new(file.file_name().unwrap_or_default().to_string_lossy()).strong());
                                            });
                                        }
                                        ui.add_space(8.0);
                                        if ui.link(RichText::new("Change file").color(accent_color).size(12.0)).clicked() {
                                            if let Some(file) = FileDialog::new().set_directory("/").pick_file() {
                                                self.files.clear();
                                                self.files.push(file);
                                            }
                                        }
                                    }
                                });
                            });

                        ui.add_space(16.0);

                        // online users section
                        ui.label(RichText::new("Online").color(text_dim).size(12.0));
                        ui.add_space(4.0);

                        // fetch users when file is selected
                        if !self.files.is_empty() && self.active_users_list.is_empty() {
                            if let Err(e) = self.to_ws.try_send(
                                WebSocketMessage::GetActiveUsersList(self.nickname.clone()),
                            ) {
                                self.tx
                                    .try_send(AppEvent::FatalError(anyhow!(e).context(
                                        "Failed to send WebSocket message to pipe",
                                    )))
                                    .ok();
                            }
                        }

                        if self.active_users_list.is_empty() {
                            egui::Frame::new()
                                .fill(bg_card)
                                .corner_radius(8.0)
                                .inner_margin(16.0)
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(RichText::new("No one else is online").color(text_dim).size(13.0));
                                    });
                                });
                        } else {
                            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                                for user in &self.active_users_list {
                                    egui::Frame::new()
                                        .fill(bg_card)
                                        .corner_radius(8.0)
                                        .inner_margin(12.0)
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new("○").color(text_dim).size(10.0));
                                                ui.label(RichText::new(user).size(13.0));
                                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                    let has_file = !self.files.is_empty();
                                                    let btn = egui::Button::new(
                                                        RichText::new("Send →")
                                                            .color(if has_file { Color32::WHITE } else { text_dim })
                                                            .size(12.0)
                                                    )
                                                        .fill(if has_file { accent_color } else { bg_dark })
                                                        .corner_radius(6.0);
                                                    if ui.add_enabled(has_file, btn).clicked() {
                                                        self.selected_file = self.files[0].clone();
                                                        let Ok(abs_path) = std::path::absolute(&self.selected_file) else {
                                                            self.tx.try_send(AppEvent::FatalError(anyhow!("Invalid path"))).ok();
                                                            return;
                                                        };
                                                        let file_name = abs_path.file_name()
                                                            .and_then(|a| a.to_str())
                                                            .unwrap_or_default()
                                                            .to_string();

                                                        if let Err(e) = self.to_ws.try_send(WebSocketMessage::PrepareFile {
                                                            recipient: user.clone(),
                                                            abs_path,
                                                            file_name
                                                        }) {
                                                            self.tx
                                                                .try_send(AppEvent::FatalError(
                                                                    anyhow!(e).context("failed to send websocket msg"),
                                                                ))
                                                                .ok();
                                                        }
                                                    }
                                                });
                                            });
                                        });
                                    ui.add_space(4.0);
                                }
                            });
                        }

                        preview_files_being_dropped(ctx);

                        ctx.input(|i| {
                            if !i.raw.dropped_files.is_empty() {
                                self.dropped_files.clone_from(&i.raw.dropped_files);
                            }
                        });
                    }
                }

                self.toasts.show(ctx);
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(e) = self
            .to_ws
            .try_send(WebSocketMessage::DisconnectUser(self.nickname.clone()))
        {
            self.tx
                .try_send(AppEvent::FatalError(
                    anyhow!(e).context("Disconnect send failed"),
                ))
                .ok();
        }
    }
}

async fn process_message(msg: Message, tx: Sender<AppEvent>) {
    if let Message::Text(bytes) = msg {
        match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
            Ok(websocket_msg) => match websocket_msg {
                WebSocketMessage::RegisterSuccess => {
                    tx.send(AppEvent::RegisterSuccess).await.ok();
                }
                WebSocketMessage::ErrorDeserializingJson(e) => {
                    tx.send(AppEvent::FatalError(
                        anyhow!(e).context("Server JSON error"),
                    ))
                    .await
                    .ok();
                }
                WebSocketMessage::ActiveUsersList(active_users_list) => {
                    tx.send(AppEvent::UpdateActiveUsersList(active_users_list))
                        .await
                        .ok();
                }
                WebSocketMessage::ReceiveFile { file_name, ticket } => {
                    let ticket = match BlobTicket::from_str(&ticket) {
                        Ok(t) => t,
                        Err(e) => {
                            tx.send(AppEvent::FatalError(
                                anyhow!(e).context("Error parsing ticket to BlobTicket"),
                            ))
                                .await
                                .ok();
                            return;
                        }
                    };

                    tx.send(AppEvent::DownloadFile { ticket, file_name })
                        .await
                        .ok();
                }
                _ => {}
            },
            Err(e) => {
                tx.send(AppEvent::FatalError(
                    anyhow!(e).context("Message parse failed"),
                ))
                .await
                .ok();
            }
        }
    }
}

fn preview_files_being_dropped(ctx: &egui::Context) {
    use std::fmt::Write as _;

    if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
        let text = ctx.input(|i| {
            let mut text = String::new();
            for file in &i.raw.hovered_files {
                if let Some(path) = &file.path && let Some(name) = path.file_name() {
                    write!(text, "{}\n", name.to_string_lossy()).ok();
                }
            }
            text
        });

        let painter = ctx.layer_painter(LayerId::new(
            egui::Order::Foreground,
            Id::new("file_drop_overlay"),
        ));
        let screen_rect = ctx.viewport_rect();

        painter.rect_filled(
            screen_rect,
            0.0,
            Color32::from_rgba_unmultiplied(30, 30, 30, 230),
        );

        let accent_color = Color32::from_rgb(79, 140, 255);
        let center = screen_rect.center();

        let inner_rect = screen_rect.shrink(24.0);
        painter.rect_stroke(
            inner_rect,
            12.0,
            Stroke::new(2.0, accent_color),
            egui::StrokeKind::Inside,
        );

        let icon_pos = center - egui::vec2(0.0, 30.0);
        painter.text(
            icon_pos,
            Align2::CENTER_CENTER,
            "📁",
            egui::FontId::proportional(48.0),
            Color32::WHITE,
        );

        let text_pos = center + egui::vec2(0.0, 30.0);
        painter.text(
            text_pos,
            Align2::CENTER_CENTER,
            if text.is_empty() {
                "Drop file here".to_string()
            } else {
                text
            },
            egui::FontId::proportional(16.0),
            Color32::WHITE,
        );
    }
}
