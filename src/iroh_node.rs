use anyhow::Result;
use iroh::{Endpoint, SecretKey};
use iroh_blobs::{store::mem::MemStore, BlobsProtocol};

pub struct IrohNode {
    pub endpoint: Endpoint,
    pub store: MemStore,
    pub blobs: BlobsProtocol,
}

impl IrohNode {
    pub async fn new() -> Result<Self> {
        let mut builder = Endpoint::builder();
        let secret_key = SecretKey::generate(&mut rand::rng());
        builder = builder.secret_key(secret_key);

        let endpoint = builder.bind().await?;
        let store = MemStore::new();
        let blobs = BlobsProtocol::new(&store, None);

        Ok(Self {
            endpoint,
            store,
            blobs,
        })
    }
}
