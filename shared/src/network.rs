use anyhow::Result;
use tokio::sync::mpsc;

use crate::{app_events::AppEvent, websocket_messages::WebSocketMessage};

pub trait Network {
    fn send(&self, event: AppEvent) -> Result<()>;
    fn send_ws(&self, ws_msg: WebSocketMessage) -> Result<()>;
    fn try_recv(&mut self) -> Option<AppEvent>;
    fn clone_tx(&self) -> mpsc::UnboundedSender<AppEvent>;
}
