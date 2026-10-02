//! The `registry` role of `https://wyvrn.app/documentation/1.0`: turns a received
//! `query`, `request` or `spec-request` into its reply (or a problem report). Pure --
//! no I/O -- so it's tested without a network.

use didcomm_agent::{features, Features, Received};
use serde_json::{json, Map, Value};

use crate::index::{Index, ProtocolDoc, SourceRef};

pub const DOCUMENTATION: &str = "https://wyvrn.app/documentation/1.0";
pub const QUERY: &str = "https://wyvrn.app/documentation/1.0/query";
pub const CATALOG: &str = "https://wyvrn.app/documentation/1.0/catalog";
pub const REQUEST: &str = "https://wyvrn.app/documentation/1.0/request";
pub const RESPONSE: &str = "https://wyvrn.app/documentation/1.0/response";
pub const SPEC_REQUEST: &str = "https://wyvrn.app/documentation/1.0/spec-request";
pub const SPEC_RESPONSE: &str = "https://wyvrn.app/documentation/1.0/spec-response";

/// What a registry discloses: the standard protocols plus `documentation/1.0`.
pub fn features() -> Features {
    Features::standard().with_protocol(DOCUMENTATION, &["registry"])
}

pub struct Registry {
    pub index: Index,
    pub max_query_limit: usize,
}

/// A problem report reply: `(code, comment, args)`.
struct Problem(&'static str, &'static str, Vec<String>);

fn invalid(what: &str) -> Problem {
    Problem("e.p.msg.invalid", "Invalid request: {1}", vec![what.to_string()])
}

impl Registry {
    /// The reply to a `documentation/1.0` request, or `None` if `received` isn't one.
    pub fn handle(&self, received: &Received) -> Option<Value> {
        let body = &received.message["body"];
        let result = match received.message_type() {
            QUERY => self.query(body).map(|b| (CATALOG, b)),
            REQUEST => self.request(body).map(|b| (RESPONSE, b)),
            SPEC_REQUEST => self.spec_request(body).map(|b| (SPEC_RESPONSE, b)),
            _ => return None,
        };
        Some(match result {
            Ok((reply_type, body)) => received.reply(reply_type, body),
            Err(Problem(code, comment, args)) => {
                let args: Vec<&str> = args.iter().map(String::as_str).collect();
                received.problem_report(code, comment, &args)
            }
        })
    }

    fn query(&self, body: &Value) -> Result<Value, Problem> {
        let pattern = optional_str(body, "match")?;
        let text = optional_str(body, "text")?.map(str::to_lowercase);
        let statuses = optional_strings(body, "status")?;
        let tags = optional_strings(body, "tags")?;
        let limit = optional_uint(body, "limit")?.unwrap_or(50).clamp(1, self.max_query_limit.max(1));
        let offset = optional_uint(body, "offset")?.unwrap_or(0);

        let matching: Vec<&ProtocolDoc> = self
            .index
            .protocols
            .values()
            .filter(|d| pattern.is_none_or(|p| features::matches(p, &d.piuri)))
            .filter(|d| statuses.as_ref().is_none_or(|s| s.iter().any(|s| s.eq_ignore_ascii_case(&d.status))))
            .filter(|d| tags.as_ref().is_none_or(|t| t.iter().any(|t| d.tags.contains(t))))
            .filter(|d| {
                text.as_deref().is_none_or(|text| {
                    d.title.to_lowercase().contains(text)
                        || d.summary.as_deref().unwrap_or_default().to_lowercase().contains(text)
                        || d.tags.iter().any(|t| t.to_lowercase().contains(text))
                })
            })
            .collect();
        let entries: Vec<Value> = matching
            .iter()
            .skip(offset)
            .take(limit)
            .map(|d| {
                let mut entry = json!({
                    "piuri": d.piuri,
                    "title": d.title,
                    "status": d.status,
                    "tags": d.tags,
                    "has_schemas": d.has_schemas(),
                });
                if let Some(summary) = &d.summary {
                    entry["summary"] = json!(summary);
                }
                entry
            })
            .collect();
        Ok(json!({"total": matching.len(), "offset": offset, "entries": entries}))
    }

    fn request(&self, body: &Value) -> Result<Value, Problem> {
        let requested = optional_str(body, "piuri")?.ok_or_else(|| invalid("body.piuri is required"))?;
        let wanted_sections = optional_strings(body, "sections")?;
        let include_messages = match &body["messages"] {
            Value::Null => true,
            Value::Bool(b) => *b,
            _ => return Err(invalid("body.messages must be a boolean")),
        };
        let doc = self.index.resolve_protocol(requested).ok_or_else(|| {
            Problem(
                "e.p.not-found.protocol",
                "No documentation for protocol {1}",
                vec![requested.to_string()],
            )
        })?;

        let sections = match &wanted_sections {
            None => doc.sections.iter().collect::<Vec<_>>(),
            Some(ids) => {
                if let Some(missing) = ids.iter().find(|id| !doc.sections.iter().any(|s| &s.id == *id)) {
                    return Err(Problem("e.p.not-found.section", "No section {1}", vec![missing.clone()]));
                }
                doc.sections.iter().filter(|s| ids.contains(&s.id)).collect()
            }
        };

        let mut reply = Map::new();
        reply.insert("piuri".into(), json!(doc.piuri));
        reply.insert("title".into(), json!(doc.title));
        reply.insert("status".into(), json!(doc.status));
        for (key, value) in [("summary", &doc.summary), ("publisher", &doc.publisher), ("license", &doc.license)] {
            if let Some(value) = value {
                reply.insert(key.into(), json!(value));
            }
        }
        reply.insert("tags".into(), json!(doc.tags));
        if !doc.authors.is_empty() {
            let authors: Vec<Value> = doc
                .authors
                .iter()
                .map(|a| {
                    let mut author = json!({"name": a.name});
                    if let Some(email) = a.email.as_deref().filter(|e| !e.trim().is_empty()) {
                        author["email"] = json!(email);
                    }
                    author
                })
                .collect();
            reply.insert("authors".into(), json!(authors));
        }
        if !doc.roles.is_empty() {
            reply.insert("roles".into(), json!(doc.roles));
        }
        reply.insert("source".into(), source_json(&doc.source));
        reply.insert(
            "available_sections".into(),
            json!(doc
                .sections
                .iter()
                .map(|s| json!({"id": s.id, "title": s.title, "level": s.level}))
                .collect::<Vec<_>>()),
        );
        reply.insert(
            "sections".into(),
            json!(sections
                .iter()
                .map(|s| json!({"id": s.id, "title": s.title, "markdown": s.markdown}))
                .collect::<Vec<_>>()),
        );
        if include_messages {
            let messages: Vec<Value> = doc
                .messages
                .iter()
                .map(|m| {
                    let mut message = json!({"type": m.message_type, "examples": m.examples});
                    if let Some(schema) = &m.schema {
                        message["schema"] = schema.clone();
                    }
                    message
                })
                .collect();
            reply.insert("messages".into(), json!(messages));
        }
        Ok(Value::Object(reply))
    }

    fn spec_request(&self, body: &Value) -> Result<Value, Problem> {
        let version = optional_str(body, "version")?;
        let section = optional_str(body, "section")?;
        let spec = self.index.resolve_spec(version).ok_or_else(|| {
            Problem(
                "e.p.not-found.spec-version",
                "No DIDComm Messaging spec version {1}",
                vec![version.unwrap_or("(latest)").to_string()],
            )
        })?;
        let mut reply = json!({"version": spec.version, "title": spec.title, "source": source_json(&spec.source)});
        match section {
            None => {
                reply["toc"] = json!(spec
                    .sections
                    .iter()
                    .map(|s| json!({"id": s.id, "title": s.title, "level": s.level}))
                    .collect::<Vec<_>>());
            }
            Some(id) => {
                let s = spec
                    .sections
                    .iter()
                    .find(|s| s.id == id)
                    .ok_or_else(|| Problem("e.p.not-found.section", "No section {1}", vec![id.to_string()]))?;
                reply["section"] = json!({"id": s.id, "title": s.title, "markdown": s.markdown});
            }
        }
        Ok(reply)
    }
}

fn source_json(source: &SourceRef) -> Value {
    let mut value = json!({"name": source.name, "path": source.path});
    if let Some(revision) = &source.revision {
        value["revision"] = json!(revision);
    }
    value
}

fn optional_str<'a>(body: &'a Value, key: &str) -> Result<Option<&'a str>, Problem> {
    match &body[key] {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        _ => Err(invalid(&format!("body.{key} must be a string"))),
    }
}

fn optional_strings(body: &Value, key: &str) -> Result<Option<Vec<String>>, Problem> {
    match &body[key] {
        Value::Null => Ok(None),
        Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .map(Some)
            .ok_or_else(|| invalid(&format!("body.{key} must be an array of strings"))),
        _ => Err(invalid(&format!("body.{key} must be an array of strings"))),
    }
}

fn optional_uint(body: &Value, key: &str) -> Result<Option<usize>, Problem> {
    match &body[key] {
        Value::Null => Ok(None),
        v => v
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| invalid(&format!("body.{key} must be a non-negative integer"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{parse_protocol, SpecDoc};
    use crate::markdown;

    fn registry() -> Registry {
        let mut index = Index::default();
        let readme = "---\ntitle: Ping Pong\npiuri: https://example.org/ping-pong/1.0\nstatus: Production\nsummary: Pings and pongs.\ntags: [testing]\n---\n## Roles\n`pinger`, `ponger`\n## Basic Walkthrough\nPing, then pong.\n```json\n{\"id\": \"1\", \"type\": \"https://example.org/ping-pong/1.0/ping\"}\n```\n";
        let source = SourceRef { name: "test".into(), path: "ping-pong/1.0/readme.md".into(), revision: Some("abc".into()) };
        let (mut doc, _) = parse_protocol(readme, source.clone(), None).unwrap();
        doc.messages[0].schema = Some(json!({"title": "ping"}));
        index.protocols.insert(doc.piuri.clone(), doc);
        let (other, _) = parse_protocol(
            "---\ntitle: Other\npiuri: https://didcomm.org/other/2.0\nstatus: Demonstrated\n---\n",
            source.clone(),
            None,
        )
        .unwrap();
        index.protocols.insert(other.piuri.clone(), other);
        index.specs.insert(
            "2.1".into(),
            SpecDoc {
                version: "2.1".into(),
                title: "Spec v2.1".into(),
                source,
                sections: markdown::sections("## Message Headers\nheaders\n### `id`\nids"),
            },
        );
        Registry { index, max_query_limit: 10 }
    }

    fn received(message_type: &str, body: Value) -> Received {
        Received {
            message: json!({"id": "req-1", "type": message_type, "body": body}),
            sender: Some("did:example:requester".into()),
            recipient_kid: "did:example:registry#key-2".into(),
        }
    }

    fn handle(message_type: &str, body: Value) -> Value {
        registry().handle(&received(message_type, body)).unwrap()
    }

    #[test]
    fn ignores_other_messages() {
        assert!(registry().handle(&received("https://didcomm.org/trust-ping/2.0/ping", json!({}))).is_none());
    }

    #[test]
    fn query_filters_and_pages() {
        let all = handle(QUERY, json!({}));
        assert_eq!(all["type"], CATALOG);
        assert_eq!(all["thid"], "req-1");
        assert_eq!(all["body"]["total"], 2);

        let by_pattern = handle(QUERY, json!({"match": "https://example.org/*"}));
        assert_eq!(by_pattern["body"]["entries"][0]["piuri"], "https://example.org/ping-pong/1.0");
        assert_eq!(by_pattern["body"]["entries"][0]["has_schemas"], true);
        assert_eq!(by_pattern["body"]["total"], 1);

        assert_eq!(handle(QUERY, json!({"text": "PONG"}))["body"]["total"], 1);
        assert_eq!(handle(QUERY, json!({"status": ["demonstrated"]}))["body"]["total"], 1);
        assert_eq!(handle(QUERY, json!({"tags": ["testing"]}))["body"]["total"], 1);

        let page = handle(QUERY, json!({"limit": 1, "offset": 1}));
        assert_eq!(page["body"]["entries"].as_array().unwrap().len(), 1);
        assert_eq!(page["body"]["offset"], 1);
        assert_eq!(page["body"]["total"], 2);
    }

    #[test]
    fn request_returns_the_definition() {
        let reply = handle(REQUEST, json!({"piuri": "https://example.org/ping-pong/1.0/ping", "sections": ["basic-walkthrough"]}));
        let body = &reply["body"];
        assert_eq!(reply["type"], RESPONSE);
        assert_eq!(body["piuri"], "https://example.org/ping-pong/1.0");
        assert_eq!(body["roles"], json!(["pinger", "ponger"]));
        assert_eq!(body["source"], json!({"name": "test", "path": "ping-pong/1.0/readme.md", "revision": "abc"}));
        assert_eq!(body["available_sections"].as_array().unwrap().len(), 2);
        assert_eq!(body["sections"], json!([{"id": "basic-walkthrough", "title": "Basic Walkthrough", "markdown": "Ping, then pong.\n```json\n{\"id\": \"1\", \"type\": \"https://example.org/ping-pong/1.0/ping\"}\n```"}]));
        assert_eq!(body["messages"][0]["schema"], json!({"title": "ping"}));
        assert_eq!(body["messages"][0]["examples"].as_array().unwrap().len(), 1);

        let lean = handle(REQUEST, json!({"piuri": "https://example.org/ping-pong/1.0", "sections": [], "messages": false}));
        assert_eq!(lean["body"]["sections"], json!([]));
        assert!(lean["body"].get("messages").is_none());
    }

    #[test]
    fn request_problems() {
        let missing = handle(REQUEST, json!({"piuri": "https://example.org/nope/1.0"}));
        assert_eq!(missing["type"], didcomm_agent::PROBLEM_REPORT);
        assert_eq!(missing["pthid"], "req-1");
        assert_eq!(missing["body"]["code"], "e.p.not-found.protocol");
        assert_eq!(missing["body"]["args"], json!(["https://example.org/nope/1.0"]));

        let section = handle(REQUEST, json!({"piuri": "https://example.org/ping-pong/1.0", "sections": ["nope"]}));
        assert_eq!(section["body"]["code"], "e.p.not-found.section");

        assert_eq!(handle(REQUEST, json!({}))["body"]["code"], "e.p.msg.invalid");
        assert_eq!(handle(REQUEST, json!({"piuri": 7}))["body"]["code"], "e.p.msg.invalid");
        assert_eq!(handle(QUERY, json!({"limit": -1}))["body"]["code"], "e.p.msg.invalid");
    }

    #[test]
    fn spec_requests() {
        let toc = handle(SPEC_REQUEST, json!({}));
        assert_eq!(toc["type"], SPEC_RESPONSE);
        assert_eq!(toc["body"]["version"], "2.1");
        assert_eq!(toc["body"]["toc"], json!([
            {"id": "message-headers", "title": "Message Headers", "level": 2},
            {"id": "id", "title": "`id`", "level": 3},
        ]));

        let section = handle(SPEC_REQUEST, json!({"version": "2.1", "section": "message-headers"}));
        assert_eq!(section["body"]["section"]["markdown"], "headers\n### `id`\nids");

        assert_eq!(handle(SPEC_REQUEST, json!({"version": "9.9"}))["body"]["code"], "e.p.not-found.spec-version");
        assert_eq!(handle(SPEC_REQUEST, json!({"section": "nope"}))["body"]["code"], "e.p.not-found.section");
    }
}
