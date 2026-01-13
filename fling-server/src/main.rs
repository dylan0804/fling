use std::{env, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, Utf8Bytes, WebSocket},
        ConnectInfo, Path, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{any, post},
    Json, Router,
};
use axum_extra::{headers::UserAgent, TypedHeader};
use dashmap::DashMap;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use message_types::WebSocketMessage;
use tokio::{
    net::TcpListener,
    sync::mpsc::{self, Receiver, Sender},
};

#[derive(Clone)]
struct AppState {
    users_list: Arc<DashMap<String, Sender<WebSocketMessage>>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            users_list: Arc::new(DashMap::new()),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = Router::new()
        .route("/ws", any(ws_handler))
        .with_state(AppState::new());

    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("{}:{}", host, port);

    let listener = TcpListener::bind(&addr).await.unwrap();

    println!("server running at {:?}", listener.local_addr());

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    _user_agent: Option<TypedHeader<UserAgent>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    println!("{addr} connected");

    ws.on_failed_upgrade(|e| {
        println!("error upgrading ws: {:?}", e);
    })
    .on_upgrade(move |socket| handle_socket(socket, state, addr))
}

async fn handle_socket(socket: WebSocket, state: AppState, who: SocketAddr) {
    let (sender, receiver) = socket.split();
    let (tx, rx) = mpsc::channel::<WebSocketMessage>(100);

    tokio::spawn(write(sender, rx));
    tokio::spawn(read(receiver, tx, state));
}

async fn write(mut sender: SplitSink<WebSocket, Message>, mut rx: Receiver<WebSocketMessage>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            WebSocketMessage::RegisterSuccess => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::RegisterSuccess.to_json().into(),
                    ))
                    .await
                    .ok();
            }
            WebSocketMessage::ActiveUsersList(active_users_list) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::ActiveUsersList(active_users_list)
                            .to_json()
                            .into(),
                    ))
                    .await
                    .ok();
            }
            WebSocketMessage::ReceiveFile(ticket) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::ReceiveFile(ticket).to_json().into(),
                    ))
                    .await
                    .ok();
            }

            // errors
            WebSocketMessage::ErrorDeserializingJson(e) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::ErrorDeserializingJson(e).to_json().into(),
                    ))
                    .await
                    .ok();
            }
            _ => {}
        }
    }
}

async fn read(mut receiver: SplitStream<WebSocket>, tx: Sender<WebSocketMessage>, state: AppState) {
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(bytes) => {
                match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
                    Ok(websocket_msg) => match websocket_msg {
                        WebSocketMessage::Register { nickname } => {
                            state.users_list.insert(nickname, tx.clone());
                            tx.send(WebSocketMessage::RegisterSuccess).await.ok();
                        }
                        WebSocketMessage::DisconnectUser(nickname) => {
                            state.users_list.remove(&nickname);
                        }
                        WebSocketMessage::GetActiveUsersList(except) => {
                            let active_users_list = state
                                .users_list
                                .iter()
                                .filter(|ref_multi| &except != ref_multi.key())
                                .map(|ref_multi| ref_multi.key().clone())
                                .collect::<Vec<String>>();

                            tx.send(WebSocketMessage::ActiveUsersList(active_users_list))
                                .await
                                .ok();
                        }
                        WebSocketMessage::SendFile { recipient, ticket } => {
                            if let Some(maybe) = state.users_list.get(&recipient) {
                                let recipient_rx = maybe.value().clone();
                                recipient_rx
                                    .send(WebSocketMessage::ReceiveFile(ticket))
                                    .await
                                    .ok();
                            }
                        }
                        _ => {}
                    },
                    Err(e) => {
                        tx.send(WebSocketMessage::ErrorDeserializingJson(e.to_string()))
                            .await
                            .ok();
                    }
                }
            }
            _ => {}
        }
    }
}
