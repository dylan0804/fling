use anyhow::Result;
use bytes::Bytes;
use eframe::wasm_bindgen::JsCast;
use futures::{channel::mpsc::UnboundedSender, stream, SinkExt, StreamExt};
use iroh::Endpoint;
use iroh_blobs::{
    api::{blobs::AddProgressItem, TempTag},
    format::collection::Collection,
    store::mem::MemStore,
    BlobsProtocol,
};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    console,
    js_sys::{Reflect, Uint8Array},
    Blob, ReadableStreamDefaultReader,
};

pub struct IrohNode {
    pub endpoint: Endpoint,
    pub blobs_protocol: BlobsProtocol,
    store: MemStore,
}

impl IrohNode {
    pub async fn new() -> Result<Self> {
        let endpoint = Endpoint::bind().await?;
        let store = MemStore::default();
        let blobs_protocol = BlobsProtocol::new(&store, None);

        Ok(Self {
            endpoint,
            store,
            blobs_protocol,
        })
    }

    pub async fn import(&self, blobs: Vec<(Blob, String)>) -> Result<TempTag> {
        let infos = futures::stream::iter(blobs)
            .map(|(blob, name)| {
                let store = self.store.clone();
                let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Bytes, std::io::Error>>();

                async move {
                    chunk_blob(blob, tx).await;
                    let progress = store.add_stream(rx).await;
                    let mut stream = progress.stream().await;
                    let temp_tag = loop {
                        if let Some(item) = stream.next().await {
                            match item {
                                AddProgressItem::Done(tt) => {
                                    break tt;
                                }
                                AddProgressItem::Error(e) => {
                                    let context = format!("Error importing {}", name);
                                    console::log_1(&context.into());
                                }
                                _ => {}
                            }
                        }
                    };
                    (name, temp_tag)
                }
            })
            .buffer_unordered(8)
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

async fn chunk_blob(blob: Blob, mut tx: UnboundedSender<Result<Bytes, std::io::Error>>) {
    spawn_local(async move {
        let stream = blob.stream();
        let reader = stream
            .get_reader()
            .dyn_into::<ReadableStreamDefaultReader>()
            .unwrap();

        loop {
            let promise = reader.read();
            let result = JsFuture::from(promise).await.unwrap();

            let done = Reflect::get(&result, &"done".into())
                .unwrap()
                .as_bool()
                .unwrap_or(true);
            if done {
                break;
            }

            let value = Reflect::get(&result, &"value".into()).unwrap();
            let vec = Uint8Array::from(value).to_vec();
            let bytes = Bytes::copy_from_slice(&vec);
            tx.send(Ok(bytes)).await.ok();
        }
        drop(tx); // probably not needed
    });
}
