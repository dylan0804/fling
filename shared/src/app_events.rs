pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess(Vec<String>),
    AddNewUser(String),
    RemoveUser(String),

    ReceivedFile(Vec<rfd::FileHandle>),
    UpdateProgressValue(f32),
    ImportStart,
    ImportDone,
    DownloadStart,
    DownloadFile(String),
    DownloadDone,

    FatalError(anyhow::Error),
}
