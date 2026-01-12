use iroh_blobs::ticket::BlobTicket;

pub enum AppEvent {
    ReadyToPublishUser,
    RegisterSuccess,

    UpdateActiveUsersList(Vec<String>),

    DownloadFile {
        ticket: BlobTicket,
        file_name: String,
    },

    FatalError(anyhow::Error),
}
