pub enum AppEvent {
    ReadyToPublishUser,
    AllSystemsGo,
    RegisterSuccess,

    FatalError(String),
}
