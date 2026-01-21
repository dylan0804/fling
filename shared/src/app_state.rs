#[derive(Debug)]
pub enum AppState {
    Connecting,
    Ready,

    PublishUser,
    WaitForRegisterConfirmation,
}
