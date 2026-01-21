use std::{env, net::SocketAddr, sync::Arc};

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        ConnectInfo, State, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::{any, Router},
};
use axum_extra::{headers::UserAgent, TypedHeader};
use dashmap::DashMap;
use futures_util::{
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use shared::websocket_messages::WebSocketMessage;
use tokio::{
    net::TcpListener,
    sync::{
        broadcast,
        mpsc::{self, Receiver, Sender},
    },
};

#[derive(Clone)]
struct AppState {
    users_list: Arc<DashMap<String, Sender<WebSocketMessage>>>,
    broadcast_tx: tokio::sync::broadcast::Sender<WebSocketMessage>,
}

impl AppState {
    fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel::<WebSocketMessage>(100);
        Self {
            users_list: Arc::new(DashMap::new()),
            broadcast_tx,
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
    .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (sender, receiver) = socket.split();
    let (tx, rx) = mpsc::channel::<WebSocketMessage>(100);

    let broadcast_rx = state.broadcast_tx.subscribe();

    tokio::spawn(write(sender, rx));
    tokio::spawn(read(receiver, tx.clone(), state));
    tokio::spawn(broadcast_read(broadcast_rx, tx));
}

async fn broadcast_read(
    mut broadcast_rx: tokio::sync::broadcast::Receiver<WebSocketMessage>,
    tx: tokio::sync::mpsc::Sender<WebSocketMessage>,
) {
    while let Ok(websocket_msg) = broadcast_rx.recv().await {
        match websocket_msg {
            WebSocketMessage::UserJoined(nickname) => {
                tx.send(WebSocketMessage::UserJoined(nickname)).await.ok();
            }
            WebSocketMessage::UserLeft(nickname) => {
                tx.send(WebSocketMessage::UserLeft(nickname)).await.ok();
            }
            _ => {}
        }
    }
}

async fn write(mut sender: SplitSink<WebSocket, Message>, mut rx: Receiver<WebSocketMessage>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            WebSocketMessage::RegisterSuccess(current_users) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::RegisterSuccess(current_users)
                            .to_json()
                            .into(),
                    ))
                    .await
                    .ok();
            }
            WebSocketMessage::UserJoined(nickname) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::UserJoined(nickname).to_json().into(),
                    ))
                    .await
                    .ok();
            }
            WebSocketMessage::UserLeft(nickname) => {
                sender
                    .send(Message::Text(
                        WebSocketMessage::UserLeft(nickname).to_json().into(),
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
    let mut current_username = String::new();
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(bytes) => {
                match serde_json::from_str::<WebSocketMessage>(bytes.as_str()) {
                    Ok(websocket_msg) => match websocket_msg {
                        WebSocketMessage::Register(nickname) => {
                            state.users_list.insert(nickname.clone(), tx.clone());
                            current_username = nickname.clone();

                            // get the already connected users
                            let current_users = state
                                .users_list
                                .iter()
                                .map(|r| r.key().clone())
                                .filter(|n| &current_username != n)
                                .collect();
                            tx.send(WebSocketMessage::RegisterSuccess(current_users))
                                .await
                                .ok();

                            // notify every1
                            state
                                .broadcast_tx
                                .send(WebSocketMessage::UserJoined(nickname))
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

    // remove user and notify the others
    state.users_list.remove(&current_username);
    state
        .broadcast_tx
        .send(WebSocketMessage::UserLeft(current_username))
        .ok();
}
