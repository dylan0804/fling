use std::{collections::HashMap, fs::File, path::PathBuf};

use anyhow::Result;
use bytes::Bytes;
use futures_util::{
    future::{join_all, try_join_all},
    stream::iter,
    StreamExt,
};
use iroh::{Endpoint, SecretKey};
use iroh_blobs::{
    api::TempTag, format::collection::Collection, store::mem::MemStore, BlobsProtocol, Hash,
};
use memmap2::Mmap;
use tokio::sync::{futures, mpsc::Sender};

use crate::events::AppEvent;

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

    pub async fn create_collection(
        &self,
        files: Vec<PathBuf>,
        tx: Sender<AppEvent>,
    ) -> Result<Hash> {
        let store = self.store.batch().await?;
        let futures = files.iter().map(|p| async {
            let file_name = p.file_name().and_then(|a| a.to_str()).unwrap_or_default();
            let file = File::open(p.clone())?;
            let content = unsafe { Mmap::map(&file)? };
            let bytes = Bytes::copy_from_slice(&content);
            let tmp_tag = store.add_bytes(bytes).await?;

            Ok::<_, anyhow::Error>((file_name, tmp_tag))
        });

        let results = join_all(futures).await;
        let mut collection_items: HashMap<&str, TempTag> = HashMap::new();

        for result in results {
            match result {
                Ok((file_name, tmp_tag)) => {
                    collection_items.insert(file_name, tmp_tag);
                }
                Err(e) => {
                    tx.send(AppEvent::FatalError(e.context("Failed to add blob")))
                        .await
                        .ok();
                }
            }
        }

        let collection_items = collection_items
            .into_iter()
            .map(|(f, tag)| (f.to_string(), tag.hash()))
            .collect::<Vec<_>>();

        let collection = Collection::from_iter(collection_items);
        let tt = collection.store(&self.store).await?;
        self.store.tags().create(tt.hash_and_format()).await?;

        Ok(tt.hash())
    }
}
