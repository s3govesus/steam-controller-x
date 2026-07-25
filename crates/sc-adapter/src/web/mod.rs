//! Local web UI: live input monitoring plus profile/remap configuration.
//!
//! Deliberately loopback-bound by default (see `--web-bind` in `main.rs`)
//! since this controls real hardware input. Bridges the synchronous HID
//! read loop to this async server with plain `std`/`tokio::sync`
//! primitives rather than making the whole adapter async — see
//! `LiveState`/`AppState` and how `main.rs` feeds them.

use std::net::SocketAddr;
use std::sync::{mpsc, Arc, RwLock};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sc_config::{LogicalButton, Profile, ProfileStore, XInputButton};
use sc_hid::Motor;
use sc_protocol::PadState;
use sc_xinput::XInputState;
use serde_json::{json, Value};
use tokio::sync::watch;

const INDEX_HTML: &str = include_str!("index.html");

/// Latest decoded state, broadcast from the main HID loop to any connected
/// browsers. `None` until the first report arrives.
#[derive(Clone, Default)]
pub struct LiveState {
    pub raw: Option<PadState>,
    pub mapped: Option<XInputState>,
}

#[derive(Clone)]
pub struct AppState {
    pub live: watch::Receiver<LiveState>,
    pub profile: Arc<RwLock<Profile>>,
    pub store: Arc<ProfileStore>,
    pub rumble_tx: mpsc::Sender<(Motor, u8)>,
}

pub async fn run(bind: SocketAddr, state: AppState) -> anyhow::Result<()> {
    let app = Router::new()
        .route("/", get(index))
        .route("/ws", get(ws_handler))
        .route("/api/logical-buttons", get(logical_buttons))
        .route("/api/xinput-buttons", get(xinput_buttons))
        .route(
            "/api/active-profile",
            get(get_active_profile).put(put_active_profile),
        )
        .route("/api/active-profile/save", post(save_active_profile))
        .route("/api/profiles", get(list_profiles))
        .route(
            "/api/profiles/{name}",
            get(get_profile).delete(delete_profile),
        )
        .route("/api/profiles/{name}/load", post(load_profile))
        .route("/api/rumble/{motor}", post(test_rumble))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "web UI listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| stream_state(socket, state.live))
}

async fn stream_state(mut socket: WebSocket, mut live: watch::Receiver<LiveState>) {
    loop {
        if live.changed().await.is_err() {
            return; // main loop shut down
        }
        let current = live.borrow_and_update().clone();
        let payload = json!({
            "raw": current.raw.as_ref().map(pad_state_json),
            "mapped": current.mapped.as_ref().map(xinput_state_json),
        });
        if socket.send(Message::Text(payload.to_string().into())).await.is_err() {
            return; // browser disconnected
        }
    }
}

fn pad_state_json(state: &PadState) -> Value {
    let mut buttons = serde_json::Map::new();
    for lb in LogicalButton::ALL {
        let name = serde_json::to_value(lb).unwrap().as_str().unwrap().to_string();
        buttons.insert(name, json!(state.buttons.contains(lb.flag())));
    }
    json!({
        "sequence": state.sequence,
        "buttons": buttons,
        "left_trigger": state.left_trigger,
        "right_trigger": state.right_trigger,
        "left_stick": { "x": state.left_stick.x, "y": state.left_stick.y },
        "right_stick": { "x": state.right_stick.x, "y": state.right_stick.y },
        "left_pad": { "x": state.left_pad.x, "y": state.left_pad.y },
        "right_pad": { "x": state.right_pad.x, "y": state.right_pad.y },
        "grip": state.grip,
        "imu": { "accel": state.imu.accel, "gyro": state.imu.gyro },
    })
}

fn xinput_state_json(state: &XInputState) -> Value {
    let mut buttons = serde_json::Map::new();
    for xb in XInputButton::ALL {
        let name = serde_json::to_value(xb).unwrap().as_str().unwrap().to_string();
        buttons.insert(name, json!(state.buttons.contains(xb.bit())));
    }
    json!({
        "buttons": buttons,
        "left_trigger": state.left_trigger,
        "right_trigger": state.right_trigger,
        "thumb_lx": state.thumb_lx,
        "thumb_ly": state.thumb_ly,
        "thumb_rx": state.thumb_rx,
        "thumb_ry": state.thumb_ry,
    })
}

async fn logical_buttons() -> Json<Value> {
    Json(serde_json::to_value(LogicalButton::ALL).unwrap())
}

async fn xinput_buttons() -> Json<Value> {
    Json(serde_json::to_value(XInputButton::ALL).unwrap())
}

fn internal_err(e: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn get_active_profile(State(state): State<AppState>) -> Json<Profile> {
    Json(state.profile.read().unwrap().clone())
}

/// Live-preview a profile change: updates the in-memory active profile
/// immediately (visible in the next `/ws` tick's `mapped` state), without
/// touching disk. Pair with `POST /api/active-profile/save` to persist.
async fn put_active_profile(
    State(state): State<AppState>,
    Json(profile): Json<Profile>,
) -> StatusCode {
    *state.profile.write().unwrap() = profile;
    StatusCode::OK
}

async fn save_active_profile(
    State(state): State<AppState>,
) -> Result<StatusCode, (StatusCode, String)> {
    let profile = state.profile.read().unwrap().clone();
    state.store.save(&profile).map_err(internal_err)?;
    state
        .store
        .set_active_name(&profile.name)
        .map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn list_profiles(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    state.store.list().map(Json).map_err(internal_err)
}

async fn get_profile(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Profile>, (StatusCode, String)> {
    state.store.load(&name).map(Json).map_err(internal_err)
}

/// Loads a saved profile from disk, makes it the active in-memory profile,
/// and records it as active for next launch.
async fn load_profile(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Profile>, (StatusCode, String)> {
    let profile = state.store.load(&name).map_err(internal_err)?;
    state.store.set_active_name(&name).map_err(internal_err)?;
    *state.profile.write().unwrap() = profile.clone();
    Ok(Json(profile))
}

async fn delete_profile(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    state.store.delete(&name).map_err(internal_err)?;
    Ok(StatusCode::OK)
}

async fn test_rumble(
    State(state): State<AppState>,
    Path(motor): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let motor = match motor.to_ascii_lowercase().as_str() {
        "a" => Motor::A,
        "b" => Motor::B,
        "c" => Motor::C,
        "d" => Motor::D,
        other => return Err((StatusCode::BAD_REQUEST, format!("unknown motor {other:?}"))),
    };
    state
        .rumble_tx
        .send((motor, 0xff))
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::OK)
}
