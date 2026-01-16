use std::{collections::HashMap, fs::File, path::PathBuf};

use anyhow::Result;
use bytes::Bytes;
use futures_util::future::join_all;
use iroh::Endpoint;
use iroh_blobs::{
    api::{tags::TagInfo, Store, TempTag},
    format::collection::Collection,
    store::{fs::FsStore, mem::MemStore},
    BlobsProtocol, Hash,
};
use memmap2::Mmap;
use tokio::{io::AsyncReadExt, sync::mpsc::Sender};

use crate::events::AppEvent;

const CHUNK_SIZE: usize = 1024 * 1024 * 32;

pub struct IrohNode {
    pub endpoint: Endpoint,
    pub store: FsStore,
    pub blobs: BlobsProtocol,
}

impl IrohNode {
    pub async fn new(download_dir: PathBuf) -> Result<Self> {
        let endpoint = Endpoint::bind().await?;
        let store = FsStore::load(download_dir).await?;
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
        let batcher = self.store.batch().await?;
        let futures = files.iter().map(|p| async {
            let file_name = p.file_name().and_then(|a| a.to_str()).unwrap_or_default();
            let tag_info = self.store.blobs().add_path(p.clone()).await?;

            Ok::<_, anyhow::Error>((file_name, tag_info))
        });

        let results = join_all(futures).await;
        let mut collection_items: HashMap<&str, TagInfo> = HashMap::new();

        for result in results {
            match result {
                Ok((file_name, tag_info)) => {
                    collection_items.insert(file_name, tag_info);
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
            .map(|(f, tag)| (f.to_string(), tag.hash))
            .collect::<Vec<_>>();

        let collection = Collection::from_iter(collection_items);
        let tt = collection.store(&self.store).await?;
        self.store.tags().create(tt.hash_and_format()).await?;

        Ok(tt.hash())
    }
}
