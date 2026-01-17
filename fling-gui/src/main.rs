

use std::{collections::HashSet, path::PathBuf};

use anyhow::{anyhow, Context};
use eframe::{CreationContext};
use egui::{vec2, Align2, Color32, CornerRadius, Id, LayerId, ProgressBar, RichText, Stroke, Vec2, Widget};
use egui_toast::{ToastKind, Toasts};
use futures_util::{SinkExt, StreamExt};
use iroh::{protocol::Router};
use iroh_blobs::{api::{remote::GetProgressItem, Store}, format::collection::Collection, get::request::get_hash_seq_and_sizes, ticket::BlobTicket, BlobFormat};
use message_types::WebSocketMessage;
use names::{Generator, Name};
use rfd::FileDialog;
use serde_json::{self};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::{events::AppEvent, iroh_node::IrohNode, state::AppState};

mod events;
mod iroh_node;
mod state;
mod toast;

const WS_URL: &str = "wss://fling-server.fly.dev/ws";

macro_rules! try_or_continue {
    ($result:expr, $tx:expr, $msg:expr) => {
       match $result {
            Ok(val) => val,
            Err(e) => {
                $tx.send(AppEvent::FatalError(anyhow!(e).context($msg))).await.ok();
                continue;
            }
        } 
    };
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

    eframe::run_native(
        "Fling",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

pub struct MyApp {
    app_state: AppState,
    nickname: String,
    users: HashSet<String>,
    toasts: Toasts,
    files: Vec<PathBuf>,
    tx: Sender<AppEvent>,
    rx: Receiver<AppEvent>,
    to_ws: Sender<WebSocketMessage>,
    download_dir: PathBuf,
    is_downloading: bool,
    is_importing: bool,
    progress: f32
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
            users: HashSet::new(),
            nickname: String::new(),
            download_dir: PathBuf::new(),
            is_downloading: false,
            is_importing: false,
            progress: 0.,
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

        let online_icon = egui_material_icons::icon_text(egui_material_icons::icons::ICON_CIRCLE)
            .color(Color32::from_rgb(80, 200, 120))
            .size(12.0);

        // top panel
        egui::TopBottomPanel::top("header")
            .frame(egui::Frame::new().fill(bg_dark).inner_margin(12.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(online_icon.clone());
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
                        AppEvent::RegisterSuccess(current_users) => {
                            let users = HashSet::from_iter(current_users);
                            self.users = users;
                            self.app_state = AppState::Ready;
                        }
                        AppEvent::AddNewUser(nickname) => {
                            if nickname != self.nickname {
                                self.users.insert(nickname);
                            }
                        }
                        AppEvent::RemoveUser(nickname) => {
                            self.users.remove(&nickname);
                        }
                        AppEvent::UpdateProgressValue(value) => {
                            self.progress = value;
                        }
                        AppEvent::ImportStart => self.is_importing = true,
                        AppEvent::ImportDone => self.is_importing = false,
                        AppEvent::DownloadStart => self.is_downloading = true,
                        AppEvent::DownloadFile(ticket) => {
                            self.to_ws.try_send(WebSocketMessage::DownloadFile(ticket)).ok();
                        }
                        AppEvent::DownloadDone => self.is_downloading = false,
                        AppEvent::FatalError(e) => {
                            self.show_toast(format!("{e:#}"), ToastKind::Error);
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
                        let ctx_clone = ctx.clone();
                        let nickname = self.nickname.clone();
                        tokio::spawn(async move {
                            let ws_init = async {
                                let ws_stream = connect_async(WS_URL)
                                    .await
                                    .context("WebSocket connection failed")?;

                                let (sender, receiver) = ws_stream.0.split();
                                Ok::<_, anyhow::Error>((sender, receiver))
                            };
                            
                            let download_dir_1 = download_dir.clone();
                            let iroh_init = async {
                                let iroh_node = IrohNode::new(download_dir_1, nickname)
                                    .await
                                    .context("Iroh node initialization failed")?;

                                let router = Router::builder(iroh_node.endpoint.clone())
                                    .accept(iroh_blobs::ALPN, iroh_node.blobs.clone())
                                    .spawn();

                                Ok((iroh_node, router))
                            };

                            let download_dir = download_dir.clone(); 
                            let ctx_clone = ctx_clone.clone();
                            tokio::spawn(async move {
                                match tokio::try_join!(ws_init, iroh_init) {
                                    Ok(((mut sender, mut receiver), (iroh_node, router))) => {
                                        // get ws msg
                                        let tx_clone = tx.clone();
                                        let ctx_clone_1 = ctx_clone.clone();
                                        tokio::spawn(async move {
                                            loop {
                                                match receiver.next().await {
                                                    Some(Ok(msg)) => process_message(msg, tx_clone.clone(), ctx_clone_1.clone()).await,
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
                                        let ctx_clone_2 = ctx_clone.clone();
                                        tokio::spawn(async move {
                                            while let Some(websocket_msg) = ws_receiver.recv().await {
                                                match websocket_msg {
                                                    WebSocketMessage::PrepareFile {
                                                        recipient,
                                                        files,
                                                    } => {
                                                        tx_clone.send(AppEvent::ImportStart).await.ok();
                                                        ctx_clone_2.request_repaint();
                                                        let tt = try_or_continue!(
                                                            iroh_node.import(files, tx_clone.clone()).await,
                                                            tx_clone,
                                                            "Failed to import file(s)"
                                                        );
                                                        tx_clone.send(AppEvent::ImportDone).await.ok();
                                                        ctx_clone_2.request_repaint();

                                                        let ticket = BlobTicket::new(
                                                            iroh_node.endpoint.addr(),
                                                            tt.hash(),
                                                            BlobFormat::HashSeq,
                                                        );

                                                        let json = WebSocketMessage::SendFile { recipient, ticket }.to_json();

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
                                                    WebSocketMessage::DownloadFile(ticket) => {
                                                        let downloader = iroh_node.store.downloader(&iroh_node.endpoint);
                                                        match downloader.download(ticket.hash(), Some(ticket.addr().id)).await {
                                                            Ok(_) =>  {
                                                                let download_dir = download_dir.clone();
                                                                let local_info = try_or_continue!(
                                                                    iroh_node.store.remote().local(ticket.hash_and_format()).await, 
                                                                    tx_clone, 
                                                                    "Failed to get local info"
                                                                );

                                                                if !local_info.is_complete() {
                                                                    let connection = try_or_continue!(
                                                                        iroh_node.endpoint.connect(ticket.addr().id, iroh_blobs::ALPN).await, 
                                                                        tx_clone, 
                                                                        "Failed to connect to sender"
                                                                    );
                                                                    let (_, size) = try_or_continue!(
                                                                        get_hash_seq_and_sizes(&connection, &ticket.hash(), 1024 * 1024 * 1024 * 5, None).await,
                                                                        tx_clone,
                                                                        "Failed to get file(s) size"
                                                                    );

                                                                    // skip the 1st index, bcs its the collection blob and obv we don't need it
                                                                    let actual_size = size.iter().skip(1).sum::<u64>();
                                                                    let get = iroh_node.store.remote().execute_get(connection, local_info.missing());
                                                                    let mut stream = get.stream();
                                                                    tx_clone.send(AppEvent::DownloadStart).await.ok();
                                                                    while let Some(item) = stream.next().await {
                                                                        match item {
                                                                            GetProgressItem::Progress(b) => {
                                                                                let value = b as f32 / actual_size as f32;
                                                                                tx_clone.send(AppEvent::UpdateProgressValue(value)).await.ok();
                                                                                ctx_clone_2.request_repaint();
                                                                            }
                                                                            GetProgressItem::Done(_) => {
                                                                                tx_clone.send(AppEvent::DownloadDone).await.ok();
                                                                                break;
                                                                            },
                                                                            GetProgressItem::Error(e) => {
                                                                                tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Error downloading one of the files"))).await.ok();
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                                
                                                                match Collection::load(ticket.hash(), iroh_node.store.as_ref()).await {
                                                                    Ok(c) => {
                                                                        for (f, h) in c.into_iter() {
                                                                            println!("file name is {f}");
                                                                            let p = download_dir.join(f);
                                                                            if let Err(e) = iroh_node.store.blobs().export(h, p).await {
                                                                                tx_clone.send(AppEvent::FatalError(anyhow!(e).context("Error downloading file {p}: {e:?}")))
                                                                                    .await
                                                                                    .ok();
                                                                            }
                                                                        }
                                                                        iroh_node.store.tags().delete_all().await.unwrap();
                                                                    }
                                                                    Err(e) => {
                                                                        tx_clone.send(AppEvent::FatalError(e.context("Error loading collection"))).await.ok();
                                                                    }
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
                                            if let Some(file) = FileDialog::new().set_directory("/").pick_files() {
                                                self.files.extend(file);
                                            }
                                        }
                                        ui.label(RichText::new("or drag and drop").color(text_dim).size(12.0));
                                    } else {
                                        ui.add_space(8.0);
                                        let mut file_to_remove: Option<usize> = None;
                                        ui.spacing_mut().item_spacing = vec2(8., 8.);
                                        for (index, file) in self.files.iter().enumerate() {
                                            egui::Frame::default()
                                                .corner_radius(8)
                                                .fill(Color32::from_rgb(45, 45, 50))
                                                .inner_margin(vec2(8., 8.))
                                                .show(ui, |ui| {
                                                    ui.set_min_width(120.0);
                                                    ui.vertical(|ui| {
                                                        ui.horizontal(|ui| {
                                                            ui.label(RichText::new("📄").size(20.0));
                                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                                                if ui.small_button(
                                                                    RichText::new("×").size(16.0).color(Color32::from_rgb(200, 80, 80))
                                                                ).on_hover_text("Remove").clicked() {
                                                                    file_to_remove = Some(index);
                                                                }
                                                            });
                                                        });
                                                        ui.add_space(4.0);
                                                        ui.label(
                                                            RichText::new(file.file_name().unwrap_or_default().to_string_lossy())
                                                                .size(12.0)
                                                        );
                                                    });
                                                });
                                        }

                                        if let Some(index) = file_to_remove {
                                            self.files.remove(index);
                                        }

                                        ui.add_space(12.0);

                                        ui.horizontal(|ui| {
                                            if ui.link(RichText::new("+ Add more files").color(accent_color).size(12.0)).clicked() {
                                                if let Some(files) = FileDialog::new().set_directory("/").pick_files() {
                                                    self.files.extend(files);
                                                }
                                            }
                                            ui.label(RichText::new("•").color(text_dim).size(12.0));
                                            if ui.link(RichText::new("Clear all").color(accent_color).size(12.0)).clicked() {
                                                self.files.clear();
                                            }
                                        });
                                    }
                                });
                            });

                        ui.add_space(16.0);

                        // online users section
                        ui.label(RichText::new("Online").color(text_dim).size(12.0));
                        ui.add_space(4.0);

                        if self.users.is_empty() {
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
                                for user in &self.users {
                                    egui::Frame::new()
                                        .fill(bg_card)
                                        .corner_radius(8.0)
                                        .inner_margin(12.0)
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(online_icon.clone());
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
                                                        if let Err(e) = self.to_ws.try_send(WebSocketMessage::PrepareFile {
                                                            recipient: user.clone(),
                                                            files: self.files.clone(),
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
                                let dropped_files = i.raw.dropped_files.iter()
                                    .map(|d| d.path.clone().unwrap_or_default())
                                    .collect::<Vec<_>>();
                                self.files.extend(dropped_files);
                            }
                        });
                    }
                }

                // importing indicator
                if self.is_importing {
                    egui::TopBottomPanel::bottom("import_panel")
                        .frame(egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(40, 40, 40, 240))
                            .inner_margin(12.0))
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.add_space(8.0);
                                ui.label(RichText::new("Preparing files...").color(text_dim).size(11.0));
                            });
                        });
                    ctx.request_repaint();
                }

                // progress bar
                if (self.progress > 0.0 && self.progress < 1.0) || self.is_downloading {
                    egui::TopBottomPanel::bottom("progress_panel")
                        .frame(egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(40, 40, 40, 240))
                            .inner_margin(12.0))
                        .show(ctx, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new("Transferring...").color(text_dim).size(11.0));
                                ui.add_space(4.0);
                                ProgressBar::new(self.progress)
                                    .show_percentage()
                                    .ui(ui);
                            });
                        });
                }

                self.toasts.show(ctx);
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
       // delete temp dir 
       let temp_dir = self.download_dir.join(format!("fling-{}", self.nickname));
       if let Err(e) = std::fs::remove_dir_all(&temp_dir) {
            self.tx.try_send(AppEvent::FatalError(anyhow!(e).context(format!("Failed to delete temp dir {:?}, be sure to delete it yourself", temp_dir)))).ok();
       }
    }
}

async fn process_message(msg: Message, tx: Sender<AppEvent>, ctx: egui::Context) {
    if let Message::Text(bytes) = msg {
        match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
            Ok(websocket_msg) => match websocket_msg {
                WebSocketMessage::RegisterSuccess(current_users) => {
                    tx.send(AppEvent::RegisterSuccess(current_users)).await.ok();
                }
                WebSocketMessage::UserJoined(nickname) => {
                    tx.send(AppEvent::AddNewUser(nickname)).await.ok();
                }
                WebSocketMessage::UserLeft(nickname) => {
                    tx.send(AppEvent::RemoveUser(nickname)).await.ok();
                }
                WebSocketMessage::ReceiveFile(ticket ) => {
                    tx.send(AppEvent::DownloadFile(ticket))
                        .await
                        .ok();
                }
                WebSocketMessage::ErrorDeserializingJson(e) => {
                    tx.send(AppEvent::FatalError(
                        anyhow!(e).context("Server JSON error"),
                    ))
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

    ctx.request_repaint();
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
