use iroh_blobs::ticket::BlobTicket;

pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess,

    UpdateActiveUsersList(Vec<String>),

    DownloadFile(BlobTicket),

    FatalError(anyhow::Error),
}
