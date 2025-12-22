use anyhow::Result;
use iroh::Endpoint;
use iroh_blobs::{store::mem::MemStore, BlobsProtocol};

pub struct IrohNode {
    pub endpoint: Endpoint,
    pub store: MemStore,
    pub blobs: BlobsProtocol,
}

impl IrohNode {
    pub async fn new() -> Result<Self> {
        let endpoint = Endpoint::bind().await?;
        let store = MemStore::new();
        let blobs = BlobsProtocol::new(&store, None);

        Ok(Self {
            endpoint,
            store,
            blobs,
        })
    }
}
