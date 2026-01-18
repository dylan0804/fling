use std::path::PathBuf;

use iroh_blobs::ticket::BlobTicket;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WebSocketMessage {
    Register(String),
    RegisterSuccess(Vec<String>),
    UserJoined(String),
    UserLeft(String),

    PrepareFile {
        recipient: String,
        files: Vec<PathBuf>,
    },
    SendFile {
        recipient: String,
        ticket: BlobTicket,
    },
    ReceiveFile(BlobTicket),
    DownloadFile(BlobTicket),

    ErrorDeserializingJson(String),
}

impl WebSocketMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).expect("error serializing json BUG!")
    }
}
