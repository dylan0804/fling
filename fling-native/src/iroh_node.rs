use std::path::PathBuf;

use anyhow::{anyhow, Result};
use futures_util::{stream, StreamExt};
use iroh::Endpoint;
use iroh_blobs::{
    api::{
        blobs::{AddBytesOptions, AddPathOptions, AddProgressItem},
        TempTag,
    },
    format::collection::Collection,
    store::{
        fs::{self, FsStore},
        mem::MemStore,
    },
    BlobsProtocol,
};
use n0_future::BufferedStreamExt;
use shared::app_events::AppEvent;
use tokio::sync::mpsc::UnboundedSender;

pub struct IrohNode {
    pub endpoint: Endpoint,
    pub store: FsStore,
    pub blobs: BlobsProtocol,
}

impl IrohNode {
    pub async fn new(download_dir: PathBuf, path: String) -> Result<Self> {
        let endpoint = Endpoint::bind().await?;
        let store = FsStore::load(download_dir.join(format!("fling-{}", path))).await?;
        let blobs = BlobsProtocol::new(&store, None);

        Ok(Self {
            endpoint,
            store,
            blobs,
        })
    }

    pub async fn import(
        &self,
        files: Vec<PathBuf>,
        tx: UnboundedSender<AppEvent>,
    ) -> Result<TempTag> {
        let collection = files
            .into_iter()
            .map(|p| {
                let name = p
                    .file_name()
                    .and_then(|a| a.to_str())
                    .unwrap_or_default()
                    .to_string();
                (name, p)
            })
            .collect::<Vec<_>>();

        let infos = n0_future::stream::iter(collection)
            .map(|(name, path)| {
                let store = self.store.clone();
                let tx_clone = tx.clone();
                async move {
                    let import = store.add_path_with_opts(AddPathOptions {
                        path,
                        mode: iroh_blobs::api::blobs::ImportMode::TryReference,
                        format: iroh_blobs::BlobFormat::Raw,
                    });
                    let mut stream = import.stream().await;
                    let temp_tag = loop {
                        if let Some(item) = stream.next().await {
                            match item {
                                AddProgressItem::Done(tt) => {
                                    break tt;
                                }
                                AddProgressItem::Error(e) => {
                                    let context = format!("Error importing {}", name);
                                    tx_clone
                                        .send(AppEvent::FatalError(anyhow!(e).context(context)))
                                        .ok();
                                }
                                _ => {}
                            }
                        }
                    };
                    (name, temp_tag)
                }
            })
            .buffered_unordered(8)
            .collect::<Vec<_>>()
            .await;

        let collection = infos
            .into_iter()
            .map(|(name, tag)| (name, tag.hash()))
            .collect::<Collection>();
        let tt = collection.store(&self.store).await?;
        Ok(tt)
    }
}
