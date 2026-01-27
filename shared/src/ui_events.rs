use crate::websocket_messages::WebSocketMessage;

#[derive(Debug, Clone)]
pub enum UIEvent {
    Register(String),
    PrepareFile {
        recipient: String,
        files: Vec<rfd::FileHandle>,
    },
    DownloadFile(String),
}

impl UIEvent {
    pub fn to_ws(self) -> Option<WebSocketMessage> {
        match self {
            Self::Register(nickname) => Some(WebSocketMessage::Register(nickname)),
            _ => None,
        }
    }
}
