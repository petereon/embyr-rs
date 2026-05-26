//! BrowserChannel long-poll protocol handlers.
//!
//! Minimal implementation supporting:
//! - POST /channel (no SID) → create session, return JSON `{"sid": "..."}`
//! - POST /channel?SID=... → forward channel: accept message bytes, return 200
//! - GET  /channel?SID=... → back channel: return buffered events as length-prefixed frames
//! - GET  /channel (no SID) → 400 Bad Request

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

/// In-memory session store for the BrowserChannel protocol.
#[derive(Clone)]
pub struct BrowserChannelState {
    pub sessions: Arc<Mutex<HashMap<String, BrowserChannelSession>>>,
}

impl BrowserChannelState {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// A single BrowserChannel session: pending events queued for back-channel delivery.
pub struct BrowserChannelSession {
    pub sid: String,
    pub events: Vec<Vec<u8>>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
pub struct ChannelQuery {
    pub SID: Option<String>,
    #[allow(dead_code)]
    pub AID: Option<String>,
}

/// POST /channel
///
/// - No SID param → create session, return `{"sid": "..."}`.
/// - SID param present → forward channel: accept message body, enqueue for back-channel, return 200.
pub async fn browser_channel_post(
    Query(q): Query<ChannelQuery>,
    State(state): State<BrowserChannelState>,
    body: Bytes,
) -> Response {
    match q.SID {
        None => {
            let sid = Uuid::new_v4().to_string();
            state.sessions.lock().await.insert(
                sid.clone(),
                BrowserChannelSession {
                    sid: sid.clone(),
                    events: Vec::new(),
                },
            );
            Json(serde_json::json!({ "sid": sid })).into_response()
        }
        Some(sid) => {
            let mut sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&sid) {
                if !body.is_empty() {
                    session.events.push(body.to_vec());
                }
                StatusCode::OK.into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}

/// GET /channel
///
/// - No SID param → 400 Bad Request (AC-13c).
/// - SID param present → drain buffered events as length-prefixed frames, return 200.
pub async fn browser_channel_get(
    Query(q): Query<ChannelQuery>,
    State(state): State<BrowserChannelState>,
) -> Response {
    match q.SID {
        None => StatusCode::BAD_REQUEST.into_response(),
        Some(sid) => {
            let mut sessions = state.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&sid) {
                // Encode buffered events as 5-byte length-prefixed frames (gRPC framing).
                let mut payload: Vec<u8> = Vec::new();
                for event in session.events.drain(..) {
                    let len = event.len() as u32;
                    payload.push(0x00); // not compressed
                    payload.extend_from_slice(&len.to_be_bytes());
                    payload.extend_from_slice(&event);
                }
                (StatusCode::OK, payload).into_response()
            } else {
                StatusCode::NOT_FOUND.into_response()
            }
        }
    }
}
