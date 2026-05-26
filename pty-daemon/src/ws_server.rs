//! WebSocket server for remote terminal access (iOS / LAN clients)
//!
//! Runs on a dedicated tokio runtime thread alongside the main kqueue loop.
//! Accepts WebSocket connections on `0.0.0.0:7685` (configurable via `PTYD_WS_PORT`).
//!
//! Protocol (same JSON as Unix socket control channel):
//! - Control messages: JSON text frames (List, Create, Ping, Kill, WinsizeUpdate, Detach)
//! - After Attach: PTY output → binary frames, client input → binary frames
//!
//! Key difference from Unix socket:
//! - No fd-passing — daemon proxies PTY I/O over WebSocket
//! - Multiple clients can attach to the same session simultaneously

use crate::protocol::{Request, Response};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

/// Default WebSocket port
const DEFAULT_WS_PORT: u16 = 7685;

/// Channel message from WebSocket client → daemon main loop
#[derive(Debug)]
pub enum WsCommand {
    /// Control request from a WebSocket client
    Request {
        client_id: u64,
        request: Request,
        reply_tx: tokio::sync::oneshot::Sender<Response>,
    },
    /// WebSocket client disconnected (may need crash-detach)
    Disconnected { client_id: u64 },
    /// Input data from an attached WebSocket client → PTY
    Input {
        session_id: Uuid,
        data: Vec<u8>,
    },
}

/// Channel message from daemon → WebSocket client (PTY output)
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// PTY output data to send as binary frame
    PtyOutput(Vec<u8>),
    /// Session ended (child exited, killed, etc.)
    SessionEnded,
}

/// Per-session WebSocket subscriber: sends PTY output to attached WS clients
#[derive(Clone)]
pub struct WsSubscriber {
    tx: mpsc::UnboundedSender<WsEvent>,
    pub client_id: u64,
}

/// Shared state between the kqueue main loop and WebSocket server
pub struct WsSharedState {
    /// Session subscribers: session_id → list of WebSocket subscribers
    pub subscribers: HashMap<Uuid, Vec<WsSubscriber>>,
    /// Next client_id counter
    next_client_id: u64,
    /// Track which sessions each WS client is attached to
    pub client_sessions: HashMap<u64, Uuid>,
}

impl WsSharedState {
    pub fn new() -> Self {
        Self {
            subscribers: HashMap::new(),
            next_client_id: 1,
            client_sessions: HashMap::new(),
        }
    }

    pub fn next_client_id(&mut self) -> u64 {
        let id = self.next_client_id;
        self.next_client_id += 1;
        id
    }

    /// Add a subscriber for a session, returns the event receiver
    pub fn subscribe(
        &mut self,
        session_id: Uuid,
        client_id: u64,
    ) -> mpsc::UnboundedReceiver<WsEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        let subscriber = WsSubscriber { tx, client_id };
        self.subscribers
            .entry(session_id)
            .or_default()
            .push(subscriber);
        self.client_sessions.insert(client_id, session_id);
        rx
    }

    /// Remove a subscriber
    pub fn unsubscribe(&mut self, session_id: &Uuid, client_id: u64) {
        if let Some(subs) = self.subscribers.get_mut(session_id) {
            subs.retain(|s| s.client_id != client_id);
            if subs.is_empty() {
                self.subscribers.remove(session_id);
            }
        }
        self.client_sessions.remove(&client_id);
    }

    /// Broadcast PTY output to all WebSocket subscribers of a session
    pub fn broadcast(&self, session_id: &Uuid, data: &[u8]) {
        if let Some(subs) = self.subscribers.get(session_id) {
            for sub in subs {
                let _ = sub.tx.send(WsEvent::PtyOutput(data.to_vec()));
            }
        }
    }

    /// Notify all subscribers that a session ended
    pub fn notify_session_ended(&mut self, session_id: &Uuid) {
        if let Some(subs) = self.subscribers.remove(session_id) {
            for sub in &subs {
                let _ = sub.tx.send(WsEvent::SessionEnded);
                self.client_sessions.remove(&sub.client_id);
            }
        }
    }

    /// Check if a session has any WebSocket subscribers
    pub fn has_subscribers(&self, session_id: &Uuid) -> bool {
        self.subscribers
            .get(session_id)
            .map_or(false, |s| !s.is_empty())
    }
}

/// Start the WebSocket server on a background thread.
///
/// Returns:
/// - `WsCommand` receiver: the main kqueue loop reads commands from this
/// - `Arc<Mutex<WsSharedState>>`: shared state for subscriber management
pub fn start_ws_server() -> (
    std::sync::mpsc::Receiver<WsCommand>,
    Arc<Mutex<WsSharedState>>,
) {
    let port = std::env::var("PTYD_WS_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(DEFAULT_WS_PORT);

    // Use std::sync::mpsc for the command channel (main loop is sync)
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<WsCommand>();
    let shared_state = Arc::new(Mutex::new(WsSharedState::new()));
    let shared_state_clone = Arc::clone(&shared_state);

    std::thread::Builder::new()
        .name("ws-server".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("failed to create tokio runtime for ws-server");

            rt.block_on(async move {
                let listener = match TcpListener::bind(("0.0.0.0", port)).await {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("[ws-server] failed to bind 0.0.0.0:{port}: {e}");
                        return;
                    }
                };

                eprintln!("[ws-server] listening on 0.0.0.0:{port}");

                loop {
                    match listener.accept().await {
                        Ok((stream, addr)) => {
                            eprintln!("[ws-server] new connection from {addr}");
                            let cmd_tx = cmd_tx.clone();
                            let shared = Arc::clone(&shared_state_clone);

                            tokio::spawn(async move {
                                if let Err(e) =
                                    handle_ws_connection(stream, cmd_tx, shared).await
                                {
                                    eprintln!("[ws-server] connection error: {e}");
                                }
                            });
                        }
                        Err(e) => {
                            eprintln!("[ws-server] accept error: {e}");
                        }
                    }
                }
            });
        })
        .expect("failed to spawn ws-server thread");

    (cmd_rx, shared_state)
}

/// Handle a single WebSocket connection
async fn handle_ws_connection(
    stream: tokio::net::TcpStream,
    cmd_tx: std::sync::mpsc::Sender<WsCommand>,
    shared: Arc<Mutex<WsSharedState>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws_stream = tokio_tungstenite::accept_async(stream).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let client_id = {
        let mut state = shared.lock().unwrap();
        state.next_client_id()
    };

    eprintln!("[ws-server] client {client_id} connected");

    // Track whether this client is in attached (streaming) mode
    let mut attached_session: Option<Uuid> = None;
    let mut event_rx: Option<mpsc::UnboundedReceiver<WsEvent>> = None;

    loop {
        // If attached, multiplex between WS input and PTY output events
        if let Some(ref mut rx) = event_rx {
            tokio::select! {
                // PTY output → WebSocket
                event = rx.recv() => {
                    match event {
                        Some(WsEvent::PtyOutput(data)) => {
                            if ws_write.send(Message::Binary(data.into())).await.is_err() {
                                break;
                            }
                        }
                        Some(WsEvent::SessionEnded) => {
                            // Send a JSON notification and exit attached mode
                            let msg = serde_json::json!({
                                "type": "SessionEnded",
                                "session_id": attached_session.unwrap().to_string()
                            });
                            let _ = ws_write
                                .send(Message::Text(msg.to_string().into()))
                                .await;
                            // Unsubscribe
                            if let Some(sid) = attached_session.take() {
                                let mut state = shared.lock().unwrap();
                                state.unsubscribe(&sid, client_id);
                            }
                            event_rx = None;
                        }
                        None => {
                            // Channel closed
                            break;
                        }
                    }
                }
                // WebSocket input → PTY or control
                msg = ws_read.next() => {
                    match msg {
                        Some(Ok(Message::Binary(data))) => {
                            // Binary frame = PTY input
                            if let Some(sid) = attached_session {
                                let _ = cmd_tx.send(WsCommand::Input {
                                    session_id: sid,
                                    data: data.into(),
                                });
                            }
                        }
                        Some(Ok(Message::Text(text))) => {
                            // Text frame = JSON control message (e.g., Detach, WinsizeUpdate)
                            match handle_text_message(
                                &text,
                                client_id,
                                &cmd_tx,
                                &shared,
                                &mut attached_session,
                                &mut event_rx,
                                &mut ws_write,
                            )
                            .await
                            {
                                Ok(should_continue) => {
                                    if !should_continue {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    let err_resp = Response::Error {
                                        message: format!("parse error: {e}"),
                                    };
                                    let _ = ws_write
                                        .send(Message::Text(
                                            serde_json::to_string(&err_resp).unwrap().into(),
                                        ))
                                        .await;
                                }
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(Message::Ping(data))) => {
                            let _ = ws_write.send(Message::Pong(data)).await;
                        }
                        Some(Ok(_)) => {} // Pong, Frame — ignore
                        Some(Err(_)) => break,
                    }
                }
            }
        } else {
            // Not attached — only process control messages
            match ws_read.next().await {
                Some(Ok(Message::Text(text))) => {
                    match handle_text_message(
                        &text,
                        client_id,
                        &cmd_tx,
                        &shared,
                        &mut attached_session,
                        &mut event_rx,
                        &mut ws_write,
                    )
                    .await
                    {
                        Ok(should_continue) => {
                            if !should_continue {
                                break;
                            }
                        }
                        Err(e) => {
                            let err_resp = Response::Error {
                                message: format!("parse error: {e}"),
                            };
                            let _ = ws_write
                                .send(Message::Text(
                                    serde_json::to_string(&err_resp).unwrap().into(),
                                ))
                                .await;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(Message::Ping(data))) => {
                    let _ = ws_write.send(Message::Pong(data)).await;
                }
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            }
        }
    }

    // Cleanup: unsubscribe from any attached session
    if let Some(sid) = attached_session {
        let mut state = shared.lock().unwrap();
        state.unsubscribe(&sid, client_id);
    }
    let _ = cmd_tx.send(WsCommand::Disconnected { client_id });

    eprintln!("[ws-server] client {client_id} disconnected");
    Ok(())
}

/// Handle a JSON text message from a WebSocket client.
///
/// Returns `Ok(true)` to continue, `Ok(false)` to close the connection.
async fn handle_text_message<S>(
    text: &str,
    client_id: u64,
    cmd_tx: &std::sync::mpsc::Sender<WsCommand>,
    shared: &Arc<Mutex<WsSharedState>>,
    attached_session: &mut Option<Uuid>,
    event_rx: &mut Option<mpsc::UnboundedReceiver<WsEvent>>,
    ws_write: &mut S,
) -> Result<bool, String>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let request: Request = serde_json::from_str(text).map_err(|e| e.to_string())?;

    // Shutdown is a special case — the WS client can request daemon shutdown
    if matches!(&request, Request::Shutdown) {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let _ = cmd_tx.send(WsCommand::Request {
            client_id,
            request,
            reply_tx,
        });
        if let Ok(resp) = reply_rx.await {
            let _ = ws_write
                .send(Message::Text(
                    serde_json::to_string(&resp).unwrap().into(),
                ))
                .await;
        }
        return Ok(false);
    }

    // Detach: clean up subscription before forwarding
    if let Request::Detach { session_id, .. } = &request {
        if attached_session.as_ref() == Some(session_id) {
            let mut state = shared.lock().unwrap();
            state.unsubscribe(session_id, client_id);
            *attached_session = None;
            *event_rx = None;
        }
    }

    // Attach: handled specially — we set up the streaming subscription
    let is_attach = matches!(&request, Request::Attach { .. });

    // Forward request to the main loop
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let _ = cmd_tx.send(WsCommand::Request {
        client_id,
        request,
        reply_tx,
    });

    let resp = match reply_rx.await {
        Ok(r) => r,
        Err(_) => Response::Error {
            message: "daemon did not respond".to_string(),
        },
    };

    // If attach succeeded, set up streaming
    if is_attach {
        if let Response::AttachReady { session_id, .. } = &resp {
            let rx = {
                let mut state = shared.lock().unwrap();
                state.subscribe(*session_id, client_id)
            };
            *attached_session = Some(*session_id);
            *event_rx = Some(rx);
        }
    }

    // Send response
    let _ = ws_write
        .send(Message::Text(
            serde_json::to_string(&resp).unwrap().into(),
        ))
        .await;

    Ok(true)
}
