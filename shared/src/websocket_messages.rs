use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum WebSocketMessage {
    Register(String),
    RegisterSuccess(Vec<String>),
    UserJoined(String),
    UserLeft(String),

    SendFile {
        recipient: String,
        ticket: String,
    },
    ReceiveFile(String),
    ErrorDeserializingJson(String),
}

impl WebSocketMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self).expect("error serializing json BUG!")
    }
}
