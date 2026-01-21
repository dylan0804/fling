use iroh_blobs::ticket::BlobTicket;

pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess(Vec<String>),
    AddNewUser(String),
    RemoveUser(String),

    UpdateProgressValue(f32),
    ImportStart,
    ImportDone,
    DownloadStart,
    DownloadFile(BlobTicket),
    DownloadDone,

    FatalError(anyhow::Error),
}
