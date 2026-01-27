use anyhow::Result;

use crate::{app_events::AppEvent, ui_events::UIEvent};

pub trait Network {
    fn send(&self, event: AppEvent);
    fn send_ws(&self, ws_msg: UIEvent) -> Result<()>;
    fn try_recv(&mut self) -> Option<AppEvent>;

    fn open_file_dialog(&mut self);
}
