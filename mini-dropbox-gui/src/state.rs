use tokio::sync::mpsc::Receiver;

#[derive(Debug)]
pub enum AppState {
    OnStartup(Option<Receiver<String>>),
    Connecting,
    Ready,

    WebSocketReady,
    IrohReady,

    PublishUser,
    WaitForRegisterConfirmation,
}
