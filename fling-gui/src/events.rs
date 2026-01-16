use iroh_blobs::ticket::BlobTicket;

pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess,
    AddNewUser(String),

    DownloadFile(BlobTicket),

    FatalError(anyhow::Error),
}
