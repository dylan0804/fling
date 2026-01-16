use egui::WidgetText;
use egui_toast::{Toast, ToastKind, ToastOptions};

use crate::MyApp;

impl MyApp {
    pub fn show_toast(&mut self, text: impl Into<WidgetText>, kind: ToastKind) {
        self.toasts.add(Toast {
            text: text.into(),
            kind,
            options: ToastOptions::default()
                .duration_in_seconds(4.)
                .show_progress(true),
            ..Default::default()
        });
    }
}
