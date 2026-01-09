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
    pub fn to_json(self) -> String {
        match self {
            WebSocketMessage::Register { nickname } => {
                serde_json::to_string(&WebSocketMessage::Register { nickname }).unwrap()
            }
            WebSocketMessage::DisconnectUser(nickname) => {
                serde_json::to_string(&WebSocketMessage::DisconnectUser(nickname)).unwrap()
            }
            WebSocketMessage::RegisterSuccess => {
                serde_json::to_string(&WebSocketMessage::RegisterSuccess).unwrap()
            }
            WebSocketMessage::GetActiveUsersList(except) => {
                serde_json::to_string(&WebSocketMessage::GetActiveUsersList(except)).unwrap()
            }
            WebSocketMessage::SendFile {
                recipient,
                ticket,
                file_name,
            } => serde_json::to_string(&WebSocketMessage::SendFile {
                recipient,
                ticket,
                file_name,
            })
            .unwrap(),
            WebSocketMessage::ReceiveFile { file_name, ticket } => {
                serde_json::to_string(&WebSocketMessage::ReceiveFile { file_name, ticket }).unwrap()
            }
            _ => "".into(),
        }
    }
}
