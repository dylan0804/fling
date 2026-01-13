use message_types::WebSocketMessage;
use tokio::sync::mpsc::Receiver;

#[derive(Debug)]
pub enum AppState {
    OnStartup(Option<Receiver<WebSocketMessage>>),
    Connecting,
    Ready,

    PublishUser,
    WaitForRegisterConfirmation,
}
