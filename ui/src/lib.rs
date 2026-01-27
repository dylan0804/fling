use std::path::PathBuf;

use anyhow::anyhow;
use eframe::CreationContext;
use egui::{ahash::{HashSet, HashSetExt}, vec2, Align2, Color32, CornerRadius, Id, LayerId, ProgressBar, RichText, Stroke, Vec2, Widget};
use egui_toast::{ToastKind, Toasts};
use rfd::FileHandle;
use shared::{app_events::AppEvent, app_state::AppState, network::Network, ui_events::UIEvent, websocket_messages::WebSocketMessage};

mod toast;

pub struct UI<N> {
    network: N,
    app_state: AppState,
    nickname: String,
    users: HashSet<String>,
    toasts: Toasts,
    files: Vec<rfd::FileHandle>,
    download_dir: PathBuf,
    is_downloading: bool,
    is_importing: bool,
    progress: f32,
}

impl<N: Network> UI<N> {
    pub fn new(cc: &CreationContext, nickname: String, download_dir: PathBuf, network: N) -> Self {
        egui_material_icons::initialize(&cc.egui_ctx);

        let toasts = Toasts::new()
            .anchor(egui::Align2::RIGHT_TOP, (-10., 10.))
            .order(egui::Order::Tooltip);

        Self {
            app_state: AppState::Connecting,
            files: Vec::new(),
            users: HashSet::new(),
            is_downloading: false,
            is_importing: false,
            progress: 0.,
            nickname,
            download_dir,
            network,
            toasts,
        }
    }

    fn cleanup(&mut self) {
        let temp_dir = self.download_dir.join(format!("fling-{}", self.nickname));
        if let Err(e) = std::fs::remove_dir_all(&temp_dir) {
            self.network
                .send(AppEvent::FatalError(anyhow!(e).context(format!(
                    "Failed to delete temp dir {:?}, be sure to delete it yourself",
                    temp_dir
                ))));
    
        }
    }
}

impl<N: Network> eframe::App for UI<N> {
    fn update(&mut self, ctx: &eframe::egui::Context, _frame: &mut eframe::Frame) {
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = vec2(8.0, 8.0);
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
                while let Some(app_event) = self.network.try_recv() {
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
                        AppEvent::ReceivedFile(files) => {
                            self.files.extend(files);
                        }
                        AppEvent::UpdateProgressValue(value) => {
                            self.progress = value;
                        }
                        AppEvent::ImportStart => self.is_importing = true,
                        AppEvent::ImportDone => self.is_importing = false,
                        AppEvent::DownloadStart => self.is_downloading = true,
                        AppEvent::DownloadFile(ticket) => {
                            self.network.send_ws(UIEvent::DownloadFile(ticket)).ok();
                        }
                        AppEvent::DownloadDone => self.is_downloading = false,
                        AppEvent::FatalError(e) => {
                            self.show_toast(format!("{e:#}"), ToastKind::Error);
                        }
                    }
                }

                match &mut self.app_state {
                    AppState::Connecting => {
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 3.0);
                            ui.add(egui::Spinner::new().size(32.0).color(accent_color));
                            ui.add_space(12.0);
                            ui.label(RichText::new("Connecting...").color(text_dim).size(14.0));
                        });
                    } 
                    AppState::PublishUser => {
                        if let Err(e) = self.network.send_ws(UIEvent::Register(self.nickname.clone())) {
                            self.network
                                .send(AppEvent::FatalError(
                                    anyhow!(e).context("Register send failed"),
                                ));
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
                                            self.network.open_file_dialog();
                                        }

                                        #[cfg(not(target_arch = "wasm32"))]
                                        ui.label(RichText::new("or drag and drop").color(text_dim).size(12.0));
                                    } else {
                                        ui.add_space(8.0);
                                        let mut file_to_remove: Option<usize> = None;
                                        ui.spacing_mut().item_spacing = vec2(8., 8.);
                                        for (index, file) in self.files.iter().enumerate() {
                                            let file = file.inner();

                                            #[cfg(target_arch = "wasm32")]
                                            let file_name = file.name();

                                            #[cfg(not(target_arch = "wasm32"))]
                                            let file_name = file.file_name().unwrap_or_default().to_string_lossy().to_string();

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
                                                            RichText::new(file_name)
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
                                                self.network.open_file_dialog();
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
                                                        if let Err(e) = self.network.send_ws(UIEvent::PrepareFile {
                                                            recipient: user.clone(),
                                                            files: self.files.clone(),
                                                        }) {
                                                            self.network
                                                                .send(AppEvent::FatalError(
                                                                    anyhow!(e).context("failed to send websocket msg"),
                                                                ));
                                                        }
                                                    }
                                                });
                                            });
                                        });
                                    ui.add_space(4.0);
                                }
                            });
                        }

                        #[cfg(not(target_arch = "wasm32"))]    
                        {
                            preview_files_being_dropped(ctx);
                            ctx.input(|i| {
                                if !i.raw.dropped_files.is_empty() {
                                    let dropped_files = i.raw.dropped_files.iter()
                                        .map(|d| {
                                            let pb = d.path.clone().unwrap_or_default();
                                            FileHandle::from(pb)
                                        })
                                        .collect::<Vec<_>>();
                                    self.files.extend(dropped_files);
                                }
                            });
                        }
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
                }

                // progress bar
                if (self.progress > 0.0 && self.progress < 1.0) || self.is_downloading {
                    egui::TopBottomPanel::bottom("progress_panel")
                        .frame(egui::Frame::new()
                            .fill(Color32::from_rgba_unmultiplied(40, 40, 40, 240))
                            .inner_margin(12.0))
                        .show(ctx, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new("Downloading file(s)...").color(text_dim).size(11.0));
                                ui.add_space(4.0);
                                ProgressBar::new(self.progress)
                                    .show_percentage()
                                    .ui(ui);
                            });
                        });
                }

                self.toasts.show(ctx);
                ctx.request_repaint();
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.cleanup();
    }
}

#[cfg(not(target_arch = "wasm32"))]
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
