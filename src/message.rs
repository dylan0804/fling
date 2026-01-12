use std::path::PathBuf;

use iroh_blobs::ticket::BlobTicket;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WebSocketMessage {
    Register {
        nickname: String,
    },
    DisconnectUser(String),

    RegisterSuccess,

    GetActiveUsersList(String),
    ActiveUsersList(Vec<String>),

    PrepareFile {
        recipient: String,
        abs_path: PathBuf,
        file_name: String,
    },
    SendFile {
        recipient: String,
        ticket: String,
        file_name: String,
    },
    ReceiveFile {
        ticket: String,
        file_name: String,
    },
    DownloadFile {
        ticket: BlobTicket,
        file_name: String,
    },

    ErrorDeserializingJson(String),
    UserNotFound,
}

impl WebSocketMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).expect("error serializing json BUG!")
    }
}
