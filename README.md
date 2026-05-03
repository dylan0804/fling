# fling

A peer-to-peer file transfer app built entirely in Rust. Sending files has never been easier.

## Web-based version
https://dylanch.pro/fling

## How it works

Fling uses [Iroh](https://github.com/n0-computer/iroh) for peer-to-peer connectivity, handling NAT traversal and hole punching so files travel directly between clients. A lightweight signaling (websocket) server (built with [Axum](https://github.com/tokio-rs/axum)) coordinates the initial connection handshake, but file data never passes through it.

The browser client is compiled to WebAssembly using [Trunk](https://trunkrs.dev/), so the entire server, native client, and browser client is written in Rust.

```
Sender                  Signaling Server               Receiver
  |                          |                             |
  |-------- register ------->|                             |
  |                          |<-------- register ----------|
  |<------- peer info -------|                             |
  |                          |                             |
  |<============ direct P2P connection ===================>|
  |                                                        |
  |=================== file transfer =====================>|
```

## Architecture

| Package | Description |
|---|---|
| `fling-server` | Axum-based websocket server for peer discovery and connection handshake |
| `fling-wasm` | Browser client compiled to WebAssembly via Trunk |
| `fling-native` | Native desktop/CLI client |
| `shared` | Shared types and protocol logic used across all packages |
| `ui` | Frontend interface |

## Tech stack

- **Rust** — entire stack
- **Iroh** — P2P connectivity (NAT traversal, hole punching)
- **Axum** — signaling server
- **WebAssembly (Trunk)** — browser client
- **Docker + Fly.io** — server deployment

## Running locally

**Server**
```bash
cd fling-server
cargo run
```

**Native client**
```bash
cd fling-native
cargo run
```

**Browser client**
```bash
cd fling-wasm
trunk serve
```

## Deployment

The signaling server is containerized and deployed on [Fly.io](https://fly.io). See `Dockerfile` and `fly.toml`.
