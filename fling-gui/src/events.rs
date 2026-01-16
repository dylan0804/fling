use iroh_blobs::ticket::BlobTicket;

pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess(Vec<String>),
    AddNewUser(String),

    DownloadFile(BlobTicket),

    FatalError(anyhow::Error),
}
