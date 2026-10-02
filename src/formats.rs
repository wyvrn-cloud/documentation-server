//! Attachment formats: the payloads issue-credential and present-proof carry in their
//! attachments (credentials, requests, proofs), named by a format identifier such as
//! `anoncreds/credential-offer@v1.0`. Read from a manifest (`attachment-formats/formats.toml`)
//! and the JSON Schemas next to it, one per format and message: `<id>/<message>.json`.
//! A `response` for a protocol lists the formats its messages carry.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::index::{didcomm_version, read_toml, Index};

/// One attachment format.
#[derive(Debug, Clone)]
pub struct AttachmentFormat {
    pub id: String,
    pub title: String,
    /// Where it is defined: a document id and, optionally, a section in it.
    pub document: String,
    pub section: Option<String>,
    pub uses: Vec<FormatUse>,
}

/// A message type that carries an attachment format.
#[derive(Debug, Clone)]
pub struct FormatUse {
    /// The full message type URI.
    pub message: String,
    /// DIDComm v1: the attachment decorator it goes in, e.g. `offers~attach`.
    pub attachment: Option<String>,
    /// JSON Schema for the attachment's content.
    pub schema: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    /// Message type URI → the v1 decorator its formatted attachments go in.
    #[serde(default)]
    attachments: BTreeMap<String, String>,
    #[serde(default, rename = "format")]
    formats: Vec<FormatEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FormatEntry {
    id: String,
    title: String,
    document: String,
    section: Option<String>,
    #[serde(rename = "use")]
    uses: Vec<UseEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UseEntry {
    /// The message name, e.g. `offer-credential`.
    message: String,
    /// PIURIs of the protocols whose `message` carries the format.
    protocols: Vec<String>,
}

impl Index {
    /// Load the attachment formats manifest at `path`. Problems (a missing schema, a
    /// protocol or message that isn't indexed, an unknown document or section) are
    /// warnings; the rest still loads.
    pub fn load_attachment_formats(&mut self, path: &Path) {
        let manifest: Manifest = match read_toml(path) {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(e);
                return;
            }
        };
        let dir = path.parent().unwrap_or(Path::new("."));
        let v1 = didcomm_version("1.0").expect("1.0 is a version");
        for entry in manifest.formats {
            let mut uses = Vec::new();
            for use_ in &entry.uses {
                let file = dir.join(&entry.id).join(format!("{}.json", use_.message));
                let schema: Value = match std::fs::read_to_string(&file)
                    .map_err(|e| e.to_string())
                    .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
                {
                    Ok(schema) => schema,
                    Err(e) => {
                        self.warnings.push(format!("attachment formats: {}: {e}", file.display()));
                        continue;
                    }
                };
                for piuri in &use_.protocols {
                    let message = format!("{piuri}/{}", use_.message);
                    let Some(doc) = self.protocols.get(piuri) else {
                        self.warnings.push(format!("attachment formats: {} names {piuri}, which isn't indexed", entry.id));
                        continue;
                    };
                    if doc.message(&message).is_none() {
                        self.warnings.push(format!("attachment formats: {} names {message}, which {piuri} doesn't define", entry.id));
                    }
                    let attachment = if doc.supports(&v1) {
                        let decorator = manifest.attachments.get(&message).cloned();
                        if decorator.is_none() {
                            self.warnings.push(format!("attachment formats: no [attachments] entry for {message}"));
                        }
                        decorator
                    } else {
                        None
                    };
                    uses.push(FormatUse { message, attachment, schema: schema.clone() });
                }
            }
            match self.documents.get(&entry.document) {
                None => self.warnings.push(format!("attachment formats: {} names document {}, which isn't indexed", entry.id, entry.document)),
                Some(document) => {
                    if let Some(section) = &entry.section {
                        let found = document.versions.values().any(|v| v.sections.iter().any(|s| &s.id == section));
                        if !found {
                            self.warnings.push(format!("attachment formats: {} names section {section}, which {} doesn't have", entry.id, entry.document));
                        }
                    }
                }
            }
            self.attachment_formats.push(AttachmentFormat {
                id: entry.id,
                title: entry.title,
                document: entry.document,
                section: entry.section,
                uses,
            });
        }
    }

    /// The formats `piuri`'s messages carry, each with only those uses.
    pub fn formats_for(&self, piuri: &str) -> Vec<AttachmentFormat> {
        let prefix = format!("{piuri}/");
        self.attachment_formats
            .iter()
            .filter_map(|format| {
                let uses: Vec<FormatUse> = format.uses.iter().filter(|u| u.message.starts_with(&prefix)).cloned().collect();
                (!uses.is_empty()).then(|| AttachmentFormat { uses, ..format.clone() })
            })
            .collect()
    }
}
