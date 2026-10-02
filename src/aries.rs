//! Reading [hyperledger/aries-rfcs](https://github.com/hyperledger/aries-rfcs), where
//! most DIDComm v1 protocols are defined.
//!
//! RFCs have no frontmatter: a header list under the title gives the status, authors
//! and tags, and the PIURI only appears in prose. So `mappings/aries-rfcs.toml` says
//! which RFC is which protocol, and which RFCs make up documents (DIDComm v1 itself, as
//! `spec` version `1.0`; the credential attachment formats).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::config::SourceConfig;
use crate::index::{self, group_examples, Author, Index, ProtocolDoc, SourceRef, SpecDoc};
use crate::markdown::{self, Section};

#[derive(Debug, Default, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub protocol: Vec<ManifestProtocol>,
    #[serde(default)]
    pub document: Vec<ManifestDocument>,
    #[serde(default)]
    pub ignore: Ignore,
}

#[derive(Debug, Deserialize)]
pub struct ManifestProtocol {
    pub piuri: String,
    pub rfc: String,
    #[serde(default)]
    pub substitute: BTreeMap<String, String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ManifestDocument {
    pub id: String,
    pub version: String,
    pub title: String,
    #[serde(default)]
    pub didcomm_versions: Vec<String>,
    pub rfcs: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Ignore {
    #[serde(default)]
    pub rfcs: Vec<String>,
}

/// What an RFC's header list says.
#[derive(Debug, Default)]
pub struct RfcHeader {
    pub number: Option<String>,
    pub title: String,
    pub status: Option<String>,
    pub authors: Vec<Author>,
    pub tags: Vec<String>,
}

impl Index {
    pub(crate) fn index_aries(
        &mut self,
        source: &SourceConfig,
        revisions: &HashMap<String, String>,
    ) {
        let Some(manifest_path) = &source.manifest else {
            self.warnings.push(format!("source {}: an aries-rfcs source needs a manifest", source.name));
            return;
        };
        let manifest: Manifest = match index::read_toml(manifest_path) {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(e);
                return;
            }
        };
        let revision = index::revision(source, revisions);
        for protocol in &manifest.protocol {
            for alias in &protocol.aliases {
                self.aliases.insert(alias.clone(), protocol.piuri.clone());
            }
        }

        for entry in &manifest.protocol {
            let dir = source.path.join(&entry.rfc);
            let readme = dir.join("README.md");
            let text = match std::fs::read_to_string(&readme) {
                Ok(t) => t,
                Err(e) => {
                    self.warnings.push(format!("{}: {e}", readme.display()));
                    continue;
                }
            };
            let source_ref = SourceRef {
                name: source.name.clone(),
                path: index::relative(&readme, &source.path),
                revision: revision.clone(),
            };
            let mut issues = Vec::new();
            let mut doc = parse_rfc(&text, &entry.piuri, source_ref, &json_files(&dir), &self.aliases, &entry.substitute, &mut issues);
            self.upstream_issues.extend(issues.into_iter().map(|i| format!("{}: {i}", readme.display())));
            doc.declared_didcomm_versions = source.didcomm_versions.clone().unwrap_or_default();
            if doc.messages.is_empty() {
                self.upstream_issues.push(format!("{}: no example messages of {}", readme.display(), entry.piuri));
            }
            match self.protocols.get(&entry.piuri) {
                // A page that only links to the RFC: the RFC is the definition.
                Some(existing) if existing.messages.is_empty() => {}
                Some(existing) => {
                    self.warnings.push(format!(
                        "{}: {} is already defined in {}:{}; list the RFC under [ignore] or drop that definition",
                        readme.display(),
                        entry.piuri,
                        existing.source.name,
                        existing.source.path
                    ));
                    continue;
                }
                None => {}
            }
            self.protocols.insert(entry.piuri.clone(), doc);
        }

        for document in &manifest.document {
            let mut sections = Vec::new();
            for rfc in &document.rfcs {
                let readme = source.path.join(rfc).join("README.md");
                match std::fs::read_to_string(&readme) {
                    Ok(text) => sections.extend(rfc_sections(rfc, &text)),
                    Err(e) => self.warnings.push(format!("{}: {e}", readme.display())),
                }
            }
            self.add_document(
                &document.id,
                if document.id == index::SPEC { "DIDComm Messaging Specification" } else { &document.title },
                if document.id == index::SPEC { &[] } else { &document.didcomm_versions },
                SpecDoc {
                    version: document.version.clone(),
                    title: document.title.clone(),
                    source: SourceRef { name: source.name.clone(), path: manifest_label(document), revision: revision.clone() },
                    sections,
                    didcomm_versions: document.didcomm_versions.clone(),
                },
            );
        }

        self.check_manifest_coverage(source, &manifest);
    }

    /// Every RFC with example messages must be in the manifest (as a protocol, in a
    /// document, or ignored), and every RFC the manifest names must exist.
    fn check_manifest_coverage(&mut self, source: &SourceConfig, manifest: &Manifest) {
        let named: BTreeSet<&str> = manifest
            .protocol
            .iter()
            .map(|p| p.rfc.as_str())
            .chain(manifest.document.iter().flat_map(|d| d.rfcs.iter().map(String::as_str)))
            .chain(manifest.ignore.rfcs.iter().map(String::as_str))
            .collect();
        for rfc in &named {
            if !source.path.join(rfc).join("README.md").is_file() {
                self.warnings.push(format!("aries-rfcs manifest: {rfc} doesn't exist in {}", source.name));
            }
        }
        for group in ["features", "concepts"] {
            for dir in index::read_dir_sorted(&source.path.join(group)) {
                let rfc = index::relative(&dir, &source.path);
                if named.contains(rfc.as_str()) {
                    continue;
                }
                let readme = std::fs::read_to_string(dir.join("README.md")).unwrap_or_default();
                let has_messages = markdown::code_blocks(&readme)
                    .iter()
                    .chain(json_files(&dir).iter())
                    .filter_map(|b| json5::from_str::<Value>(b).ok())
                    .any(|v| v.get("@type").is_some_and(Value::is_string));
                if has_messages {
                    self.warnings.push(format!(
                        "aries-rfcs manifest: {rfc} has example messages but isn't listed (add it as a protocol, to a document, or under [ignore])"
                    ));
                }
            }
        }
    }
}

fn manifest_label(document: &ManifestDocument) -> String {
    format!("{} RFCs: {}", document.rfcs.len(), document.rfcs.join(", "))
}

/// The contents of the `*.json` files directly in an RFC's folder (some keep their
/// example messages there).
fn json_files(dir: &Path) -> Vec<String> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect()
}

/// One RFC as a protocol definition.
pub fn parse_rfc(
    text: &str,
    piuri: &str,
    source: SourceRef,
    json_files: &[String],
    aliases: &BTreeMap<String, String>,
    substitute: &BTreeMap<String, String>,
    issues: &mut Vec<String>,
) -> ProtocolDoc {
    let header = parse_header(text);
    // The title's own section would repeat the whole RFC; serve what's under it.
    let sections: Vec<Section> = markdown::sections(text).into_iter().filter(|s| s.level > 1).collect();
    let roles = sections.iter().find(|s| s.id == "roles").map(|s| index::roles(&s.markdown)).unwrap_or_default();
    let summary = sections.iter().find(|s| s.id == "summary").map(|s| first_paragraph(&s.markdown)).filter(|s| !s.is_empty());
    let substitute: Vec<(String, String)> = substitute.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let blocks = markdown::code_blocks(text).into_iter().chain(json_files.iter().cloned());
    let messages = group_examples(piuri, blocks, aliases, &substitute, issues);
    ProtocolDoc {
        piuri: piuri.to_string(),
        title: header.title,
        status: header.status.unwrap_or_else(|| "Unknown".to_string()),
        summary,
        publisher: None,
        license: Some("Apache-2.0".to_string()),
        tags: header.tags,
        authors: header.authors,
        roles,
        source,
        sections,
        messages,
        aliases: Vec::new(),
        declared_didcomm_versions: Vec::new(),
    }
}

/// An RFC's sections for a compiled document: every id prefixed with the RFC number
/// (`rfc0008-...`), and the title's section, which holds the whole RFC, as `rfc0008`.
pub fn rfc_sections(rfc: &str, text: &str) -> Vec<Section> {
    let folder = rfc.rsplit('/').next().unwrap_or(rfc);
    let number: String = folder.chars().take_while(char::is_ascii_digit).collect();
    let prefix = format!("rfc{number}");
    let mut title_seen = false;
    markdown::sections(text)
        .into_iter()
        .map(|mut s| {
            if s.level == 1 && !title_seen {
                title_seen = true;
                s.id = prefix.clone();
            } else {
                s.id = format!("{prefix}-{}", s.id);
            }
            s
        })
        .collect()
}

/// The title (`# Aries RFC 0048: Trust Ping Protocol 1.0` → `Trust Ping Protocol 1.0`)
/// and the header list under it (`- Status: [ADOPTED](...)`, `- Authors: ...`,
/// `- Tags: ...`).
pub fn parse_header(text: &str) -> RfcHeader {
    let mut header = RfcHeader::default();
    for line in text.lines() {
        let line = line.trim();
        if let Some(title) = line.strip_prefix("# ") {
            if header.title.is_empty() {
                let (number, title) = split_title(title.trim());
                header.number = number;
                header.title = title;
            }
            continue;
        }
        if line.starts_with("## ") {
            break;
        }
        let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) else { continue };
        let Some((key, value)) = item.split_once(':') else { continue };
        let value = value.trim();
        match key.trim().trim_matches('*').to_ascii_lowercase().as_str() {
            "status" => header.status = status(value),
            "authors" | "author" => header.authors = authors(value),
            "tags" => header.tags = link_texts(value),
            _ => {}
        }
    }
    header
}

/// `Aries RFC 0048: Trust Ping` / `0160: Connection Protocol` / `RFC 0780: Data URLs`
/// → the number and the rest.
fn split_title(title: &str) -> (Option<String>, String) {
    if let Some((before, after)) = title.split_once(':') {
        let digits: String = before.chars().filter(char::is_ascii_digit).collect();
        let rest = before.trim_start_matches("Aries").trim().trim_start_matches("RFC").trim();
        if digits.len() >= 3 && rest.chars().all(|c| c.is_ascii_digit()) {
            return (Some(digits), after.trim().to_string());
        }
    }
    (None, title.to_string())
}

/// `[ADOPTED](/README.md#adopted) (But should move to deprecated)` → `Adopted`.
fn status(value: &str) -> Option<String> {
    let word = link_texts(value).into_iter().next().unwrap_or_else(|| value.split_whitespace().next().unwrap_or_default().to_string());
    let word = word.trim_matches(|c: char| !c.is_ascii_alphabetic());
    let mut chars = word.chars();
    let first = chars.next()?;
    Some(first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase())
}

/// `[Ryan West](mailto:ryan@x.org), Matthew Hailstone` → names, with emails from
/// `mailto:` links.
fn authors(value: &str) -> Vec<Author> {
    let mut found = Vec::new();
    let mut rest = value;
    while !rest.trim().is_empty() {
        let trimmed = rest.trim_start_matches([',', ' ']);
        if let Some(after) = trimmed.strip_prefix('[') {
            let Some((name, after)) = after.split_once(']') else { break };
            let mut email = None;
            let mut remaining = after;
            if let Some(link) = after.strip_prefix('(') {
                if let Some((target, after)) = link.split_once(')') {
                    email = target.strip_prefix("mailto:").map(str::to_string);
                    remaining = after;
                }
            }
            found.push(Author { name: Some(name.trim().to_string()), email });
            rest = remaining;
        } else {
            let (name, after) = trimmed.split_once(',').unwrap_or((trimmed, ""));
            let name = name.trim();
            if !name.is_empty() {
                found.push(Author { name: Some(name.to_string()), email: None });
            }
            rest = after;
        }
    }
    found
}

/// The texts of `[text](target)` links.
fn link_texts(value: &str) -> Vec<String> {
    value
        .split('[')
        .skip(1)
        .filter_map(|part| part.split_once("](").map(|(text, _)| text.trim().to_string()))
        .filter(|t| !t.is_empty())
        .collect()
}

fn first_paragraph(markdown: &str) -> String {
    markdown
        .split("\n\n")
        .map(str::trim)
        .find(|p| !p.is_empty() && !p.starts_with('#'))
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RFC: &str = r#"# Aries RFC 0048: Trust Ping Protocol 1.0

- Authors: [Daniel Hardman](mailto:daniel@example.org), Matthew Hailstone
- Status: [ADOPTED](/README.md#adopted) (numerous implementations)
- Tags: [feature](/tags.md#feature), [protocol](/tags.md#protocol)

## Summary

Describe a standard way for agents to test
connectivity.

More text.

## Tutorial

### Roles

There are two parties: the `sender` and the `receiver`.

### Messages

```json
{"@type": "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/trust_ping/1.0/ping", "@id": "1"}
```

```json
{"@type": "%BASE/ping_response", "@id": "2", "~thread": {"thid": "1"}}
```
"#;

    fn source() -> SourceRef {
        SourceRef { name: "aries-rfcs".into(), path: "features/0048-trust-ping/README.md".into(), revision: None }
    }

    #[test]
    fn header() {
        let header = parse_header(RFC);
        assert_eq!(header.number.as_deref(), Some("0048"));
        assert_eq!(header.title, "Trust Ping Protocol 1.0");
        assert_eq!(header.status.as_deref(), Some("Adopted"));
        assert_eq!(header.tags, ["feature", "protocol"]);
        assert_eq!(header.authors.len(), 2);
        assert_eq!(header.authors[0].name.as_deref(), Some("Daniel Hardman"));
        assert_eq!(header.authors[0].email.as_deref(), Some("daniel@example.org"));
        assert_eq!(header.authors[1].name.as_deref(), Some("Matthew Hailstone"));
        assert_eq!(split_title("0160: Connection Protocol"), (Some("0160".into()), "Connection Protocol".into()));
        assert_eq!(split_title("RFC 0780: Data URLs"), (Some("0780".into()), "Data URLs".into()));
        assert_eq!(split_title("Something: else"), (None, "Something: else".into()));
    }

    #[test]
    fn an_rfc_as_a_protocol() {
        let substitute = BTreeMap::from([("%BASE".to_string(), "https://didcomm.org/trust_ping/1.0".to_string())]);
        let mut issues = Vec::new();
        let doc = parse_rfc(RFC, "https://didcomm.org/trust_ping/1.0", source(), &[], &BTreeMap::new(), &substitute, &mut issues);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(doc.title, "Trust Ping Protocol 1.0");
        assert_eq!(doc.status, "Adopted");
        assert_eq!(doc.summary.as_deref(), Some("Describe a standard way for agents to test connectivity."));
        assert_eq!(doc.roles, ["sender", "receiver"]);
        let types: Vec<_> = doc.messages.iter().map(|m| m.message_type.as_str()).collect();
        assert_eq!(types, ["https://didcomm.org/trust_ping/1.0/ping", "https://didcomm.org/trust_ping/1.0/ping_response"]);
        // Served as written.
        assert_eq!(doc.messages[1].examples[0]["@type"], "%BASE/ping_response");
        assert_eq!(doc.messages[0].didcomm_versions(), ["^1.0"]);
        assert!(doc.sections.iter().all(|s| s.level > 1), "the title's section is left out");
    }

    #[test]
    fn rfc_sections_are_prefixed() {
        let ids: Vec<_> = rfc_sections("features/0048-trust-ping", RFC).into_iter().map(|s| s.id).collect();
        assert_eq!(ids, ["rfc0048", "rfc0048-summary", "rfc0048-tutorial", "rfc0048-roles", "rfc0048-messages"]);
    }
}
