use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::{channel::mpsc, SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;
use warp::{ws::{Message, WebSocket}, Filter};

/* ---------------- WAMP v1 opcodes ----------------
   0 = WELCOME    [0, sessionId, protocolVersion, serverIdent]
   1 = PREFIX     [1, curie, uri]                (logged, ignored)
   2 = CALL       [2, callId, procUri, ...]      (reply with CALLRESULT [3, callId, result])
   3 = CALLRESULT [3, callId, result]
   4 = CALLERROR  [4, callId, errorUri, desc]    (unused here)
   5 = SUBSCRIBE  [5, topicUri]
   6 = UNSUBSCRIBE[6, topicUri]
   7 = PUBLISH    [7, topicUri, event]           (logged, ignored)
   8 = EVENT      [8, topicUri, event]
--------------------------------------------------- */

type ClientId = Uuid;

/* ============================ Hub ============================ */

#[derive(Clone)]
struct Hub {
    inner: Arc<Mutex<HubInner>>,
}

struct HubInner {
    // topic -> set of client ids
    subs: HashMap<String, HashSet<ClientId>>,
    // client id -> sender
    clients: HashMap<ClientId, mpsc::UnboundedSender<Message>>,
}

impl Hub {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HubInner {
                subs: HashMap::new(),
                clients: HashMap::new(),
            })),
        }
    }

    fn register_client(&self, id: ClientId, tx: mpsc::UnboundedSender<Message>) {
        let mut inner = self.inner.lock();
        inner.clients.insert(id, tx);
    }

    fn unregister_client(&self, id: ClientId) {
        let mut inner = self.inner.lock();
        inner.clients.remove(&id);
        for subs in inner.subs.values_mut() {
            subs.remove(&id);
        }
    }

    fn subscribe(&self, id: ClientId, topic: &str) {
        let mut inner = self.inner.lock();
        inner.subs.entry(topic.to_string()).or_default().insert(id);
    }

    fn unsubscribe(&self, id: ClientId, topic: &str) {
        let mut inner = self.inner.lock();
        if let Some(set) = inner.subs.get_mut(topic) {
            set.remove(&id);
            if set.is_empty() {
                inner.subs.remove(topic);
            }
        }
    }

    /// Send EVENT [8, topicUri, payload] to all subscribers of `topic`.
    fn publish_event(&self, topic: &str, payload: Value) {
        let targets: Vec<mpsc::UnboundedSender<Message>> = {
            let inner = self.inner.lock();
            inner
                .subs
                .get(topic)
                .into_iter()
                .flat_map(|set| set.iter())
                .filter_map(|cid| inner.clients.get(cid).cloned())
                .collect()
        };
        if targets.is_empty() {
            return;
        }

        let frame = json!([8, topic, payload]).to_string();
        for mut tx in targets {
            let _ = tx.start_send(Message::text(frame.clone()));
        }
    }

    /// Send a raw frame text to a single client (used for CALLRESULT).
    fn send_to_client(&self, id: ClientId, payload_text: String) {
        let tx_opt = { self.inner.lock().clients.get(&id).cloned() };
        if let Some(mut tx) = tx_opt {
            let _ = tx.start_send(Message::text(payload_text));
        }
    }
}

/* ====================== Server public API ===================== */

#[derive(Serialize)]
struct MotionPayload { x: i32, y: i32, z: i32, rx: i32, ry: i32, rz: i32 }

#[derive(Serialize)]
struct ButtonPayload { button: u32 }

#[derive(Clone)]
pub struct SpaceMouseWampServer {
    hub: Hub,
}

impl SpaceMouseWampServer {
    pub fn new() -> Self { Self { hub: Hub::new() } }

    /// Helper: wrap a property update into the payload the JS expects:
    /// [2, "<id>", "self:update", null, "<prop>", <value>]
    fn self_update(prop: &str, val: Value) -> Value {
        json!([2, Uuid::new_v4().to_string(), "self:update", Value::Null, prop, val])
    }

    pub fn publish_json_prop(&self, topic: &str, prop: &str, value: Value) {
        self.hub.publish_event(topic, Self::self_update(prop, value));
    }

    pub fn publish_motion(&self, topic: &str, x: i32, y: i32, z: i32, rx: i32, ry: i32, rz: i32) {
        let val = serde_json::to_value(MotionPayload { x, y, z, rx, ry, rz }).unwrap();
        self.hub.publish_event(topic, Self::self_update("motion", val));
    }

    pub fn publish_button(&self, topic: &str, button: u32, pressed: bool) {
        let prop = if pressed { "events.keyPress" } else { "events.keyRelease" };
        let val  = serde_json::to_value(ButtonPayload { button }).unwrap();
        self.hub.publish_event(topic, Self::self_update(prop, val));
    }

    /// WS at ws://127.0.0.1:8180/ (Caddy proxies WSS to this)
    pub async fn run(self) {
        let server_for_filter = self.clone();
        let server_filter = warp::any().map(move || server_for_filter.clone());

        let ws_route = warp::path::end()
            .and(warp::ws())
            .and(server_filter)
            .map(|ws: warp::ws::Ws, server: SpaceMouseWampServer| {
                ws.on_upgrade(move |socket| handle_client(socket, server))
            });

        let addr: std::net::SocketAddr = "127.0.0.1:8180".parse().unwrap();
        eprintln!("WS listening on ws://{}/  (behind Caddy TLS)", addr);
        warp::serve(ws_route).run(addr).await;
    }
}

/* ====================== Connection handler ===================== */

async fn handle_client(ws: WebSocket, server: SpaceMouseWampServer) {
    let id = Uuid::new_v4();
    eprintln!("CONNECT {}", id);
    let (mut sink, mut stream) = ws.split();

    // Channel for hub -> this client
    let (tx, mut rx) = mpsc::unbounded::<Message>();
    server.hub.register_client(id, tx);

    // Send WELCOME immediately: [0, session, 1, "rust-wamp"]
    let welcome = json!([0, id.to_string(), 1, "rust-wamp"]);
    if sink.send(Message::text(welcome.to_string())).await.is_err() {
        eprintln!("{}: failed to send WELCOME, closing", id);
        server.hub.unregister_client(id);
        return;
    }
    eprintln!("{} <= WELCOME", id);

    // A) forward hub -> client
    let mut sink_forward = sink;
    let forward_to_client = async move {
        while let Some(msg) = rx.next().await {
            if sink_forward.send(msg).await.is_err() {
                break;
            }
        }
    };

    // B) read client -> hub
    let server2 = server.clone();
    let read_from_client = async move {
        while let Some(Ok(msg)) = stream.next().await {
            if msg.is_close() { eprintln!("{}: CLOSE", id); break; }
            if !msg.is_text() { continue; }

            let text = msg.to_str().unwrap_or("");
            eprintln!("{} => {}", id, text);

            let Ok(val) = serde_json::from_str::<Value>(text) else {
                eprintln!("{}: non-JSON, ignore", id);
                continue;
            };
            let Some(code) = val.get(0).and_then(|c| c.as_u64()) else {
                eprintln!("{}: no opcode, ignore", id);
                continue;
            };

            match code {
                5 => {
                    // SUBSCRIBE [5, topicUri]
                    if let Some(topic) = val.get(1).and_then(|t| t.as_str()) {
                        eprintln!("{} SUBSCRIBE {}", id, topic);
                        server2.hub.subscribe(id, topic);

                        // If this is a controller instance, start spoofing motion
                        if topic.starts_with("3dconnexion:3dcontroller/") {
                            let hub = server2.hub.clone();
                            let t = topic.to_string();
                            tokio::spawn(async move {
                                loop {
                                    hub.publish_event(&t, json!([2, Uuid::new_v4().to_string(),
                                        "self:update", null, "motion",
                                        { "x": 120, "y": 0, "z": 0, "rx": 0, "ry": 0, "rz": 0 }
                                    ]));
                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                    hub.publish_event(&t, json!([2, Uuid::new_v4().to_string(),
                                        "self:update", null, "motion",
                                        { "x": -120, "y": 0, "z": 0, "rx": 0, "ry": 0, "rz": 0 }
                                    ]));
                                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                                }
                            });
                        }
                    }
                }
                6 => {
                    // UNSUBSCRIBE [6, topicUri]
                    if let Some(topic) = val.get(1).and_then(|t| t.as_str()) {
                        eprintln!("{} UNSUBSCRIBE {}", id, topic);
                        server2.hub.unsubscribe(id, topic);
                    }
                }
                1 => {
                    // PREFIX [1, curie, uri] — log + ignore
                    let curie = val.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                    let uri   = val.get(2).and_then(|v| v.as_str()).unwrap_or("?");
                    eprintln!("{} PREFIX {} => {}", id, curie, uri);
                }
                2 => {
                    // CALL [2, callId, procUri, ...] — reply with CALLRESULT [3, callId, result]
                    let call_id = val.get(1).and_then(|v| v.as_str()).unwrap_or("0");
                    let proc    = val.get(2).and_then(|v| v.as_str()).unwrap_or("?");

                    // common positional args
                    let arg0 = val.get(3).and_then(|v| v.as_str());
                    // let arg1 = val.get(4); // often "connexion" from first create
                    // let arg2 = val.get(5); // options object

                    eprintln!("{} CALL id={} proc={} arg0={:?}", id, call_id, proc, arg0);

                    let result = if proc == "3dx_rpc:create" {
                        match arg0 {
                            // First create: provide a "connexion" handle string
                            Some("3dconnexion:3dmouse") => json!({ "connexion": "wamp" }),
                            // Second create: provide a controller "instance" id (as string)
                            Some("3dconnexion:3dcontroller") => json!({ "instance": "0" }),
                            _ => json!({ "ok": true }),
                        }
                    } else if proc == "3dx_rpc:getVersion" {
                        json!({ "version": "0.7.0" })
                    } else if proc == "3dx_rpc:update" {
                        // Accept and ack updates (focus, timing, commands, images, etc.)
                        json!({ "ok": true })
                    } else {
                        // Default benign response
                        json!({ "ok": true })
                    };

                    let reply = json!([3, call_id, result]).to_string(); // CALLRESULT
                    server2.hub.send_to_client(id, reply);
                    eprintln!("{} <= CALLRESULT {}", id, call_id);
                }
                7 => {
                    // PUBLISH [7, topicUri, event] — not needed; log only
                    let topic = val.get(1).and_then(|v| v.as_str()).unwrap_or("?");
                    eprintln!("{} PUBLISH {}", id, topic);
                }
                3 => {
                    // CALLRESULT [3, callId, result] — ack for our self:update payloads.
                    // Harmless; ignore to keep logs clean.
                }
                other => {
                    eprintln!("{} unhandled opcode {} frame: {}", id, other, val);
                }
            }
        }
    };

    futures::pin_mut!(forward_to_client, read_from_client);
    futures::future::select(forward_to_client, read_from_client).await;
    server.hub.unregister_client(id);
    eprintln!("DISCONNECT {}", id);
}

/* ============================== main ============================== */

#[tokio::main]
async fn main() {
    let server = SpaceMouseWampServer::new();

    // OPTIONAL: background publisher you can keep for debugging against a known topic.
    // (It only has effect if a client actually SUBSCRIBEs to that topic.)
    let debug = server.clone();
    tokio::spawn(async move {
        let topic_guess = "3dconnexion:3dcontroller/0";
        loop {
            debug.publish_motion(topic_guess, 50, 0, 0, 0, 0, 0);
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            debug.publish_motion(topic_guess, -50, 0, 0, 0, 0, 0);
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    });

    server.run().await;
}

