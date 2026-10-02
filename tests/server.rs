//! The server end to end over HTTP on localhost, with a real DIDComm client
//! (`didcomm-agent`) as the requester, over the real sources.
//!
//! If a checkout of wyvrn-cloud/protocols sits next to this repository (or
//! `DOCUMENTATION_SCHEMAS` points at its `protocols/documentation` folder), every reply
//! is also validated against the published schemas of its documentation version.

use std::path::PathBuf;
use std::sync::Arc;

use didcomm_agent::{features, Agent, AgentError, Identity};
use documentation_server::{
    config::{Config, DidMethod},
    index::Index,
    registry::{self, Registry},
    server::{self, AppState},
};
use serde_json::{json, Value};

/// Start a server on a free port; returns its DID and base URL.
async fn start(did_method: DidMethod) -> (String, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let mut config = Config::load("config/default.toml").unwrap();
    config.server.public_url = url.clone();
    config.server.did_method = did_method;

    let (agent, did_document) = server::build_agent(&config, Identity::generate().unwrap()).unwrap();
    let did = agent.did();
    let state = Arc::new(AppState {
        agent,
        registry: Registry { index: Index::build(&config), max_query_limit: config.index.max_query_limit },
        did_document,
    });
    tokio::spawn(async move { axum::serve(listener, server::router(state)).await });
    (did, url)
}

fn client() -> Agent {
    Agent::new(Identity::generate().unwrap()).unwrap()
}

/// The published schema for a documentation message type (1.0 or 1.1), if a protocols
/// checkout is available.
fn documentation_schema(message_type: &str) -> Option<jsonschema::Validator> {
    let dir = std::env::var_os("DOCUMENTATION_SCHEMAS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../protocols/protocols/documentation"));
    let (piuri, name) = message_type.rsplit_once('/')?;
    let version = piuri.strip_prefix("https://wyvrn.app/documentation/")?;
    let path = dir.join(version).join("schemas").join(format!("{name}.json"));
    let schema: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(jsonschema::validator_for(&schema).unwrap())
}

/// Request `body` as `message_type`, check the reply against the published schema
/// (when available), and return it.
async fn ask(registry_did: &str, message_type: &str, body: Value) -> Result<Value, AgentError> {
    let reply = client().request(registry_did, &json!({"type": message_type, "body": body})).await?;
    if let Some(validator) = documentation_schema(reply.message_type()) {
        let errors: Vec<String> = validator.iter_errors(&reply.message).map(|e| e.to_string()).collect();
        assert!(errors.is_empty(), "{} doesn't match its schema: {errors:?}", reply.message_type());
    }
    Ok(reply.message)
}

#[tokio::test]
async fn query_finds_protocols() {
    let (did, _) = start(DidMethod::Peer).await;

    let catalog = ask(&did, registry::QUERY, json!({"match": "https://didcomm.org/coordinate-mediation/*"}))
        .await
        .unwrap();

    assert_eq!(catalog["type"], registry::CATALOG);
    let piuris: Vec<_> = catalog["body"]["entries"].as_array().unwrap().iter().map(|e| e["piuri"].clone()).collect();
    assert_eq!(piuris, [
        "https://didcomm.org/coordinate-mediation/1.0",
        "https://didcomm.org/coordinate-mediation/2.0",
        "https://didcomm.org/coordinate-mediation/3.0",
    ]);
    assert_eq!(catalog["body"]["entries"][2]["has_schemas"], true);
}

#[tokio::test]
async fn request_returns_sections_messages_and_schemas() {
    let (did, _) = start(DidMethod::Peer).await;

    let response = ask(
        &did,
        registry::REQUEST,
        json!({"piuri": "https://didcomm.org/messagepickup/3.0/status", "sections": ["roles"]}),
    )
    .await
    .unwrap();

    let body = &response["body"];
    assert_eq!(body["piuri"], "https://didcomm.org/messagepickup/3.0");
    assert_eq!(body["sections"].as_array().unwrap().len(), 1);
    assert!(body["sections"][0]["markdown"].as_str().unwrap().contains("`mediator`"));
    let status = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["type"] == "https://didcomm.org/messagepickup/3.0/status")
        .unwrap();
    assert_eq!(status["schema"]["required"], json!(["id", "type", "thid", "body"]));
    assert!(!status["examples"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn spec_sections_are_served() {
    let (did, _) = start(DidMethod::Peer).await;

    let toc = ask(&did, registry::SPEC_REQUEST, json!({})).await.unwrap();
    assert_eq!(toc["body"]["version"], "2.1");
    assert!(toc["body"]["toc"].as_array().unwrap().len() > 100);

    let section = ask(&did, registry::SPEC_REQUEST, json!({"version": "2.0", "section": "message-headers"}))
        .await
        .unwrap();
    assert!(section["body"]["section"]["markdown"].as_str().unwrap().contains("`thid`"));
}

#[tokio::test]
async fn documentation_1_0_requesters_still_get_1_0_replies() {
    let (did, _) = start(DidMethod::Peer).await;

    let catalog = ask(&did, "https://wyvrn.app/documentation/1.0/query", json!({"text": "mediation"})).await.unwrap();
    assert_eq!(catalog["type"], "https://wyvrn.app/documentation/1.0/catalog");
    let spec = ask(&did, "https://wyvrn.app/documentation/1.0/spec-request", json!({"version": "2.0"})).await.unwrap();
    assert_eq!(spec["type"], "https://wyvrn.app/documentation/1.0/spec-response");
    let response = ask(&did, "https://wyvrn.app/documentation/1.0/request", json!({"piuri": "https://didcomm.org/trust-ping/2.0"}))
        .await
        .unwrap();
    assert!(response["body"]["messages"][0]["schema"].is_object());
}

#[tokio::test]
async fn didcomm_versions_are_reported_and_filtered() {
    let (did, _) = start(DidMethod::Peer).await;

    let v2 = ask(&did, registry::QUERY, json!({"match": "https://didcomm.org/coordinate-mediation/*", "didcomm_version": "2.1"}))
        .await
        .unwrap();
    let entries = v2["body"]["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["piuri"] == "https://didcomm.org/coordinate-mediation/3.0"));
    assert!(entries.iter().all(|e| e["didcomm_versions"].as_array().unwrap().iter().any(|r| r == "^2.0")));

    let response = ask(&did, registry::REQUEST, json!({"piuri": "https://didcomm.org/trust-ping/2.0", "sections": []}))
        .await
        .unwrap();
    let ping = &response["body"]["messages"][0];
    assert_eq!(ping["didcomm_versions"], json!(["^2.0"]));
    assert_eq!(ping["schemas"][0]["didcomm_versions"], json!(["^2.0"]));
    assert_eq!(ping["schemas"][0]["schema"], ping["schema"]);

    let toc = ask(&did, registry::SPEC_REQUEST, json!({})).await.unwrap();
    let spec = toc["body"]["documents"].as_array().unwrap().iter().find(|d| d["id"] == "spec").unwrap().clone();
    assert_eq!(spec["versions"], json!(["2.1", "2.0", "editors-draft"]));
}

#[tokio::test]
async fn unknown_things_are_problem_reports() {
    let (did, _) = start(DidMethod::Peer).await;

    let not_found = ask(&did, registry::REQUEST, json!({"piuri": "https://didcomm.org/escrow/1.0"})).await;
    match not_found {
        Err(AgentError::Problem { code, .. }) => assert_eq!(code, "e.p.not-found.protocol"),
        other => panic!("expected a problem report, got {other:?}"),
    }

    let unsupported = ask(&did, "https://example.org/nonsense/1.0/what", json!({})).await;
    match unsupported {
        Err(AgentError::Problem { code, .. }) => assert_eq!(code, "e.p.msg.unsupported"),
        other => panic!("expected a problem report, got {other:?}"),
    }
}

#[tokio::test]
async fn discloses_the_registry_role_and_answers_pings() {
    let (did, _) = start(DidMethod::Peer).await;

    let disclose = client()
        .request(&did, &json!({
            "type": features::DISCOVER_FEATURES_QUERIES,
            "body": {"queries": [{"feature-type": "protocol", "match": "https://wyvrn.app/*"}]},
        }))
        .await
        .unwrap();
    assert_eq!(
        disclose.message["body"]["disclosures"],
        json!([
            {"feature-type": "protocol", "id": registry::DOCUMENTATION, "roles": ["registry"]},
            {"feature-type": "protocol", "id": registry::DOCUMENTATION_1_0, "roles": ["registry"]},
        ])
    );

    let pong = client().request(&did, &json!({"type": features::TRUST_PING_PING, "body": {}})).await.unwrap();
    assert_eq!(pong.message_type(), features::TRUST_PING_RESPONSE);
}

#[tokio::test]
async fn a_did_web_server_publishes_its_document() {
    let (did, url) = start(DidMethod::Web).await;
    assert!(did.starts_with("did:web:127.0.0.1%3A"), "{did}");

    let document: Value = reqwest::get(format!("{url}.well-known/did.json")).await.unwrap().json().await.unwrap();
    assert_eq!(document["id"], did);
    assert_eq!(document["keyAgreement"], json!([format!("{did}#key-2")]));
    assert_eq!(document["service"][0]["serviceEndpoint"]["uri"], url);

    let peer_did = reqwest::get(format!("{url}did")).await.unwrap().text().await.unwrap();
    assert_eq!(peer_did, did);
}

/// Against an already-running server, e.g. the container image or a deployment:
/// `DOCSERVER_URL=http://localhost:8080/ cargo test --test server -- --ignored`.
#[tokio::test]
#[ignore]
async fn smoke_test_a_running_server() {
    let url = std::env::var("DOCSERVER_URL").expect("DOCSERVER_URL");
    let did = reqwest::get(format!("{}/did", url.trim_end_matches('/'))).await.unwrap().text().await.unwrap();

    let catalog = ask(&did, registry::QUERY, json!({"text": "mediation"})).await.unwrap();
    assert!(catalog["body"]["total"].as_u64().unwrap() > 0);
    let response = ask(&did, registry::REQUEST, json!({"piuri": "https://didcomm.org/trust-ping/2.0", "sections": []}))
        .await
        .unwrap();
    assert!(response["body"]["messages"].as_array().unwrap().iter().any(|m| m.get("schema").is_some()));
    assert!(response["body"]["source"]["revision"].is_string(), "responses name their source revision");
}
