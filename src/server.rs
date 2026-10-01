//! The HTTP side: DIDComm messages on `POST /`, the DID and (for `did:web`) its
//! document on `GET`.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use didcomm_agent::{Agent, Identity, Received, PROBLEM_REPORT};
use serde_json::Value;

use crate::config::{Config, DidMethod};
use crate::registry::{self, Registry};

pub struct AppState {
    pub agent: Agent,
    pub registry: Registry,
    /// Published at `/.well-known/did.json` and `/did.json` when the server is a
    /// `did:web`.
    pub did_document: Option<Value>,
}

/// The agent for `config`: a `did:web` derived from `server.public_url`, or a
/// `did:peer:4` derived from the identity's keys and that URL.
pub fn build_agent(config: &Config, identity: Identity) -> anyhow::Result<(Agent, Option<Value>)> {
    let endpoint = &config.server.public_url;
    let (agent, document) = match config.server.did_method {
        DidMethod::Peer => (Agent::with_endpoint(identity, endpoint)?, None),
        DidMethod::Web => {
            let did = did_web_for(endpoint)?;
            let document = identity.did_document(&did, endpoint);
            (Agent::with_did(identity, &did), Some(document))
        }
    };
    Ok((agent.with_features(registry::features()), document))
}

/// The `did:web` for a URL: host (`:port` percent-encoded), then any path segments
/// joined by `:` -- `https://example.com:8443/docs/` → `did:web:example.com%3A8443:docs`.
pub fn did_web_for(url: &str) -> anyhow::Result<String> {
    let url = reqwest::Url::parse(url)?;
    let host = url.host_str().ok_or_else(|| anyhow::anyhow!("{url} has no host"))?;
    let mut did = format!("did:web:{host}");
    if let Some(port) = url.port() {
        did.push_str(&format!("%3A{port}"));
    }
    for segment in url.path_segments().into_iter().flatten().filter(|s| !s.is_empty()) {
        did.push(':');
        did.push_str(segment);
    }
    Ok(did)
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", post(receive))
        .route("/did", get(did))
        .route("/.well-known/did.json", get(did_document))
        .route("/did.json", get(did_document))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

async fn did(State(state): State<Arc<AppState>>) -> String {
    state.agent.did()
}

async fn did_document(State(state): State<Arc<AppState>>) -> Response {
    match &state.did_document {
        Some(document) => Json(document.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "this server's DID is not a did:web").into_response(),
    }
}

async fn receive(State(state): State<Arc<AppState>>, body: Bytes) -> Response {
    let received = match state.agent.receive(&body).await {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("unpack failed: {e}")).into_response(),
    };
    tracing::debug!(sender = ?received.sender, message_type = received.message_type(), "received");

    let Some(reply) = reply_to(&state, &received) else {
        return StatusCode::ACCEPTED.into_response();
    };
    match state.agent.respond(&received, &reply).await {
        Ok(Some(packed)) => ([(header::CONTENT_TYPE, "application/didcomm-encrypted+json")], packed).into_response(),
        Ok(None) => StatusCode::ACCEPTED.into_response(),
        Err(e) => {
            tracing::warn!(error = %e, "couldn't deliver reply");
            (StatusCode::BAD_GATEWAY, format!("couldn't deliver reply: {e}")).into_response()
        }
    }
}

/// The registry's answer, a standard auto-reply, or -- for anything else from an
/// identifiable sender -- a problem report naming the unsupported type. Never answers
/// an anonymous message (no key to encrypt a reply to) or a problem report (no
/// report-about-a-report loops).
fn reply_to(state: &AppState, received: &Received) -> Option<Value> {
    received.sender.as_ref()?;
    if received.message_type() == PROBLEM_REPORT {
        return None;
    }
    state
        .registry
        .handle(received)
        .or_else(|| state.agent.auto_reply(received))
        .or_else(|| {
            Some(received.problem_report(
                "e.p.msg.unsupported",
                "Unsupported message type {1}",
                &[received.message_type()],
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn did_web_derivation() {
        assert_eq!(did_web_for("https://docs.example/").unwrap(), "did:web:docs.example");
        assert_eq!(did_web_for("https://docs.example:8443/a/b/").unwrap(), "did:web:docs.example%3A8443:a:b");
    }
}
