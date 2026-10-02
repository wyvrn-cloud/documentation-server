//! The `registry` role of `https://wyvrn.app/documentation/1.1` (and 1.0): turns a
//! received `query`, `request` or `spec-request` into its reply (or a problem report).
//! Pure -- no I/O -- so it's tested without a network.
//!
//! A request is answered in its own version. 1.1's additions are all extra fields, which
//! 1.0 requesters ignore, so both get the same body.

use didcomm_agent::{features, Features, Received};
use serde_json::{json, Map, Value};

use crate::index::{self, DocumentNotFound, Index, ProtocolDoc, SourceRef, SPEC};

pub const DOCUMENTATION: &str = "https://wyvrn.app/documentation/1.1";
pub const DOCUMENTATION_1_0: &str = "https://wyvrn.app/documentation/1.0";
pub const QUERY: &str = "https://wyvrn.app/documentation/1.1/query";
pub const CATALOG: &str = "https://wyvrn.app/documentation/1.1/catalog";
pub const REQUEST: &str = "https://wyvrn.app/documentation/1.1/request";
pub const RESPONSE: &str = "https://wyvrn.app/documentation/1.1/response";
pub const SPEC_REQUEST: &str = "https://wyvrn.app/documentation/1.1/spec-request";
pub const SPEC_RESPONSE: &str = "https://wyvrn.app/documentation/1.1/spec-response";

/// What a registry discloses: the standard protocols plus both documentation versions.
pub fn features() -> Features {
    Features::standard()
        .with_protocol(DOCUMENTATION, &["registry"])
        .with_protocol(DOCUMENTATION_1_0, &["registry"])
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
    /// The reply to a `documentation` request, or `None` if `received` isn't one.
    pub fn handle(&self, received: &Received) -> Option<Value> {
        let message_type = received.message_type();
        let (piuri, name) = message_type.rsplit_once('/')?;
        if piuri != DOCUMENTATION && piuri != DOCUMENTATION_1_0 {
            return None;
        }
        let body = &received.message["body"];
        let result = match name {
            "query" => self.query(body).map(|b| ("catalog", b)),
            "request" => self.request(body).map(|b| ("response", b)),
            "spec-request" => self.spec_request(body).map(|b| ("spec-response", b)),
            _ => return None,
        };
        Some(match result {
            Ok((reply_name, body)) => received.reply(&format!("{piuri}/{reply_name}"), body),
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
        let didcomm = match optional_str(body, "didcomm_version")? {
            None => None,
            Some(v) => Some(
                index::didcomm_version(v)
                    .ok_or_else(|| invalid("body.didcomm_version must be a version like 1.0 or 2.1"))?,
            ),
        };
        let limit = optional_uint(body, "limit")?.unwrap_or(50).clamp(1, self.max_query_limit.max(1));
        let offset = optional_uint(body, "offset")?.unwrap_or(0);

        let matching: Vec<&ProtocolDoc> = self
            .index
            .protocols
            .values()
            .filter(|d| pattern.is_none_or(|p| features::matches(p, &d.piuri)))
            .filter(|d| statuses.as_ref().is_none_or(|s| s.iter().any(|s| s.eq_ignore_ascii_case(&d.status))))
            .filter(|d| tags.as_ref().is_none_or(|t| t.iter().any(|t| d.tags.contains(t))))
            .filter(|d| didcomm.as_ref().is_none_or(|v| d.supports(v)))
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
                let versions = d.didcomm_versions();
                if !versions.is_empty() {
                    entry["didcomm_versions"] = json!(versions);
                }
                if !d.aliases.is_empty() {
                    entry["aliases"] = json!(d.aliases);
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
        let versions = doc.didcomm_versions();
        if !versions.is_empty() {
            reply.insert("didcomm_versions".into(), json!(versions));
        }
        if !doc.aliases.is_empty() {
            reply.insert("aliases".into(), json!(doc.aliases));
        }
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
                    let mut message = json!({"type": m.message_type});
                    let versions = m.didcomm_versions();
                    if !versions.is_empty() {
                        message["didcomm_versions"] = json!(versions);
                    }
                    message["examples"] = json!(m.examples);
                    if let Some(schema) = m.schema() {
                        message["schema"] = schema.clone();
                        message["schemas"] = json!(m
                            .schemas
                            .iter()
                            .map(|s| json!({"didcomm_versions": [s.envelope.range()], "schema": s.schema}))
                            .collect::<Vec<_>>());
                    }
                    message
                })
                .collect();
            reply.insert("messages".into(), json!(messages));
            let formats: Vec<Value> = self
                .index
                .formats_for(&doc.piuri)
                .into_iter()
                .map(|f| {
                    let mut documentation = json!({"document": f.document});
                    if let Some(section) = &f.section {
                        documentation["section"] = json!(section);
                    }
                    let uses: Vec<Value> = f
                        .uses
                        .iter()
                        .map(|u| {
                            let mut entry = json!({"message": u.message});
                            if let Some(attachment) = &u.attachment {
                                entry["attachment"] = json!(attachment);
                            }
                            entry["schema"] = u.schema.clone();
                            entry
                        })
                        .collect();
                    json!({"format": f.id, "title": f.title, "documentation": documentation, "uses": uses})
                })
                .collect();
            if !formats.is_empty() {
                reply.insert("attachment_formats".into(), json!(formats));
            }
        }
        Ok(Value::Object(reply))
    }

    fn spec_request(&self, body: &Value) -> Result<Value, Problem> {
        let document_id = optional_str(body, "document")?;
        let version = optional_str(body, "version")?;
        let section = optional_str(body, "section")?;
        let (document, doc) = self.index.resolve_document(document_id, version).map_err(|e| match e {
            DocumentNotFound::Document => Problem(
                "e.p.not-found.document",
                "No document {1}",
                vec![document_id.unwrap_or(SPEC).to_string()],
            ),
            DocumentNotFound::Version => Problem(
                "e.p.not-found.spec-version",
                "No version {1} of this document",
                vec![version.unwrap_or("(latest)").to_string()],
            ),
        })?;
        let mut reply = json!({
            "document": document.id,
            "version": doc.version,
            "title": doc.title,
        });
        if !doc.didcomm_versions.is_empty() {
            reply["didcomm_versions"] = json!(doc.didcomm_versions);
        }
        reply["source"] = source_json(&doc.source);
        match section {
            None => {
                reply["toc"] = json!(doc
                    .sections
                    .iter()
                    .map(|s| json!({"id": s.id, "title": s.title, "level": s.level}))
                    .collect::<Vec<_>>());
                reply["documents"] = json!(self
                    .index
                    .documents
                    .values()
                    .map(|d| {
                        let mut entry = json!({"id": d.id, "title": d.title, "versions": d.versions_newest_first()});
                        if !d.didcomm_versions.is_empty() {
                            entry["didcomm_versions"] = json!(d.didcomm_versions);
                        }
                        entry
                    })
                    .collect::<Vec<_>>());
            }
            Some(id) => {
                let s = doc
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
    use crate::index::{parse_protocol, Envelope, SpecDoc, VersionedSchema};
    use crate::markdown;
    use std::collections::BTreeMap;

    const DOCUMENTATION_1_0_QUERY: &str = "https://wyvrn.app/documentation/1.0/query";

    /// Two protocols: ping-pong (DIDComm v2, with a schema) and other (DIDComm v1, with
    /// a v1 schema, reachable by its legacy prefix); the spec v2.1 and one extension.
    fn registry() -> Registry {
        let mut index = Index::default();
        let readme = "---\ntitle: Ping Pong\npiuri: https://example.org/ping-pong/1.0\nstatus: Production\nsummary: Pings and pongs.\ntags: [testing]\n---\n## Roles\n`pinger`, `ponger`\n## Basic Walkthrough\nPing, then pong.\n```json\n{\"id\": \"1\", \"type\": \"https://example.org/ping-pong/1.0/ping\"}\n```\n";
        let source = SourceRef { name: "test".into(), path: "ping-pong/1.0/readme.md".into(), revision: Some("abc".into()) };
        let (mut doc, _) = parse_protocol(readme, source.clone(), None, &BTreeMap::new()).unwrap();
        doc.messages[0].schemas = vec![VersionedSchema { envelope: Envelope::V2, schema: json!({"title": "ping"}) }];
        index.protocols.insert(doc.piuri.clone(), doc);
        let (mut other, _) = parse_protocol(
            "---\ntitle: Other\npiuri: https://didcomm.org/other/1.0\nstatus: Demonstrated\ndidcomm_versions: [\"^1.0\"]\n---\n```json\n{\"@id\": \"1\", \"@type\": \"did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/other/1.0/hello\"}\n```\n",
            source.clone(),
            None,
            &BTreeMap::new(),
        )
        .unwrap();
        other.messages[0].schemas = vec![VersionedSchema { envelope: Envelope::V1, schema: json!({"title": "hello v1"}) }];
        other.aliases = vec!["did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/other/1.0".into()];
        index.protocols.insert(other.piuri.clone(), other);
        index.add_document(
            SPEC,
            "DIDComm Messaging Specification",
            &[],
            SpecDoc {
                version: "2.1".into(),
                title: "Spec v2.1".into(),
                source: source.clone(),
                sections: markdown::sections("## Message Headers\nheaders\n### `id`\nids"),
                didcomm_versions: vec!["~2.1".into()],
            },
        );
        index.add_document(
            "extension/l10n",
            "L10n Extension",
            &["^2.0".into()],
            SpecDoc {
                version: "current".into(),
                title: "L10n Extension".into(),
                source,
                sections: markdown::sections("## Scope\nlanguages"),
                didcomm_versions: vec!["^2.0".into()],
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

    #[test]
    fn documents_are_listed_and_served() {
        let toc = handle(SPEC_REQUEST, json!({}));
        assert_eq!(toc["body"]["document"], "spec");
        assert_eq!(toc["body"]["didcomm_versions"], json!(["~2.1"]));
        assert_eq!(toc["body"]["documents"], json!([
            {"id": "extension/l10n", "title": "L10n Extension", "versions": ["current"], "didcomm_versions": ["^2.0"]},
            {"id": "spec", "title": "DIDComm Messaging Specification", "versions": ["2.1"]},
        ]));

        let scope = handle(SPEC_REQUEST, json!({"document": "extension/l10n", "section": "scope"}));
        assert_eq!(scope["body"]["document"], "extension/l10n");
        assert_eq!(scope["body"]["version"], "current");
        assert_eq!(scope["body"]["section"]["markdown"], "languages");

        let missing = handle(SPEC_REQUEST, json!({"document": "extension/nope"}));
        assert_eq!(missing["body"]["code"], "e.p.not-found.document");
        assert_eq!(missing["body"]["args"], json!(["extension/nope"]));
    }

    #[test]
    fn didcomm_versions_and_aliases() {
        let catalog = handle(QUERY, json!({}));
        let entries = catalog["body"]["entries"].as_array().unwrap();
        assert_eq!(entries[0]["didcomm_versions"], json!(["^1.0"]));
        assert_eq!(entries[0]["aliases"], json!(["did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/other/1.0"]));
        assert_eq!(entries[1]["didcomm_versions"], json!(["^2.0"]));

        let v1 = handle(QUERY, json!({"didcomm_version": "1.0"}));
        assert_eq!(v1["body"]["total"], 1);
        assert_eq!(v1["body"]["entries"][0]["piuri"], "https://didcomm.org/other/1.0");
        assert_eq!(handle(QUERY, json!({"didcomm_version": "2.1"}))["body"]["entries"][0]["piuri"], "https://example.org/ping-pong/1.0");
        assert_eq!(handle(QUERY, json!({"didcomm_version": "3.0"}))["body"]["total"], 0);
        assert_eq!(handle(QUERY, json!({"didcomm_version": "^2.0"}))["body"]["code"], "e.p.msg.invalid");

        // A legacy-prefixed message type finds the protocol; its v1 example and schema
        // are filed under the https type.
        let other = handle(REQUEST, json!({"piuri": "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/other/1.0/hello"}));
        let body = &other["body"];
        assert_eq!(body["piuri"], "https://didcomm.org/other/1.0");
        assert_eq!(body["didcomm_versions"], json!(["^1.0"]));
        let hello = &body["messages"][0];
        assert_eq!(hello["type"], "https://didcomm.org/other/1.0/hello");
        assert_eq!(hello["didcomm_versions"], json!(["^1.0"]));
        assert_eq!(hello["schema"], json!({"title": "hello v1"}));
        assert_eq!(hello["schemas"], json!([{"didcomm_versions": ["^1.0"], "schema": {"title": "hello v1"}}]));
    }

    #[test]
    fn answers_documentation_1_0_in_1_0() {
        let catalog = handle(DOCUMENTATION_1_0_QUERY, json!({}));
        assert_eq!(catalog["type"], "https://wyvrn.app/documentation/1.0/catalog");
        assert_eq!(catalog["body"]["total"], 2);
        assert!(registry().handle(&received("https://wyvrn.app/documentation/0.9/query", json!({}))).is_none());
        assert!(registry().handle(&received("https://wyvrn.app/documentation/1.1/nonsense", json!({}))).is_none());
    }
}
