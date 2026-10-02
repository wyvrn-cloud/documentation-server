//! The in-memory index of everything the registry serves, built once at startup.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::config::{Config, SourceConfig, SourceKind};
use crate::markdown::{self, Section};

/// Where a document came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub name: String,
    pub path: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Author {
    pub name: Option<String>,
    pub email: Option<String>,
}

/// One message type of a protocol, with what the registry knows about it.
#[derive(Debug, Clone, Default)]
pub struct MessageType {
    pub message_type: String,
    pub examples: Vec<Value>,
    pub schema: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ProtocolDoc {
    pub piuri: String,
    pub title: String,
    pub status: String,
    pub summary: Option<String>,
    pub publisher: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    pub authors: Vec<Author>,
    pub roles: Vec<String>,
    pub source: SourceRef,
    pub sections: Vec<Section>,
    /// In order of first appearance, then schema-only types.
    pub messages: Vec<MessageType>,
}

impl ProtocolDoc {
    pub fn has_schemas(&self) -> bool {
        self.messages.iter().any(|m| m.schema.is_some())
    }
}

#[derive(Debug, Clone)]
pub struct SpecDoc {
    pub version: String,
    pub title: String,
    pub source: SourceRef,
    pub sections: Vec<Section>,
}

#[derive(Debug, Default)]
pub struct Index {
    pub protocols: BTreeMap<String, ProtocolDoc>,
    pub specs: BTreeMap<String, SpecDoc>,
    /// Problems with this server's own setup (unreadable files, unmapped folder/PIURI
    /// mismatches, invalid or orphaned schemas, ...). Logged at startup; the index test
    /// requires none for the repository's own configuration.
    pub warnings: Vec<String>,
    /// Problems in the upstream content itself, e.g. an example that isn't even JSON5.
    /// Logged and reported, never fatal: the example still reaches requesters inside
    /// its section's Markdown, just not under `messages`.
    pub upstream_issues: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Frontmatter {
    title: Option<String>,
    piuri: Option<String>,
    status: Option<String>,
    summary: Option<String>,
    publisher: Option<String>,
    license: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    authors: Option<Vec<Author>>,
}

#[derive(Debug, Default, Deserialize)]
struct Mappings {
    #[serde(default)]
    piuri: HashMap<String, String>,
}

/// `(base, major, minor)` of a PIURI like `https://didcomm.org/name/2.1`.
pub fn piuri_version(piuri: &str) -> Option<(&str, u32, u32)> {
    let (base, version) = piuri.rsplit_once('/')?;
    let (major, minor) = version.split_once('.')?;
    Some((base, major.parse().ok()?, minor.parse().ok()?))
}

impl Index {
    pub fn build(config: &Config) -> Self {
        let mut index = Index::default();
        let mappings = match &config.index.mappings {
            Some(path) => match read_toml::<Mappings>(path) {
                Ok(m) => m,
                Err(e) => {
                    index.warnings.push(e);
                    Mappings::default()
                }
            },
            None => Mappings::default(),
        };

        let revisions = match &config.index.revisions {
            Some(path) => match read_toml::<HashMap<String, String>>(path) {
                Ok(r) => r,
                Err(e) => {
                    index.warnings.push(e);
                    HashMap::new()
                }
            },
            None => HashMap::new(),
        };

        // Overlay schemas first; schemas shipped with a protocol replace them.
        let mut schemas = BTreeMap::new();
        for dir in &config.index.schemas {
            index.load_schemas(dir, &mut schemas);
        }
        for source in &config.sources {
            match source.kind {
                SourceKind::ProtocolRegistry => index.index_registry(source, &revisions, &mappings, &mut schemas),
                SourceKind::DidcommSpec => index.index_spec(source, &revisions),
            }
        }
        index.attach_schemas(schemas);
        index
    }

    /// The protocol a `request` names: an exact PIURI, a message type URI within one,
    /// or the highest minor version under the same major version.
    pub fn resolve_protocol(&self, requested: &str) -> Option<&ProtocolDoc> {
        let candidates = [Some(requested), requested.rsplit_once('/').map(|(p, _)| p)];
        for piuri in candidates.into_iter().flatten() {
            if let Some(doc) = self.protocols.get(piuri) {
                return Some(doc);
            }
            let Some((base, major, _)) = piuri_version(piuri) else { continue };
            let best = self
                .protocols
                .values()
                .filter_map(|doc| piuri_version(&doc.piuri).map(|v| (v, doc)))
                .filter(|((b, m, _), _)| *b == base && *m == major)
                .max_by_key(|((_, _, minor), _)| *minor);
            if let Some((_, doc)) = best {
                return Some(doc);
            }
        }
        None
    }

    /// The spec version a `spec-request` names; `None` = the latest numbered version.
    pub fn resolve_spec(&self, version: Option<&str>) -> Option<&SpecDoc> {
        match version {
            Some(v) => self.specs.get(v),
            None => self
                .specs
                .values()
                .filter_map(|s| {
                    let (major, minor) = s.version.split_once('.')?;
                    Some(((major.parse::<u32>().ok()?, minor.parse::<u32>().ok()?), s))
                })
                .max_by_key(|(v, _)| *v)
                .map(|(_, s)| s),
        }
    }

    fn index_registry(
        &mut self,
        source: &SourceConfig,
        revisions: &HashMap<String, String>,
        mappings: &Mappings,
        schemas: &mut BTreeMap<String, Value>,
    ) {
        let revision = revision(source, revisions);
        let mut readmes = Vec::new();
        for name in read_dir_sorted(&source.path) {
            for version in read_dir_sorted(&name) {
                let readme = version.join("readme.md");
                if readme.is_file() {
                    readmes.push(readme);
                }
                self.load_schemas(&version.join("schemas"), schemas);
            }
        }
        let mut seen_paths = Vec::new();
        for readme in readmes {
            let folder = relative(readme.parent().unwrap_or(&readme), &source.path);
            seen_paths.push(folder.clone());
            let text = match std::fs::read_to_string(&readme) {
                Ok(t) => t,
                Err(e) => {
                    self.warnings.push(format!("{}: {e}", readme.display()));
                    continue;
                }
            };
            let source_ref = SourceRef {
                name: source.name.clone(),
                path: relative(&readme, &source.path),
                revision: revision.clone(),
            };
            let folder_piuri = source.piuri_base.as_ref().map(|base| format!("{base}{folder}"));
            match parse_protocol(&text, source_ref, folder_piuri.as_deref()) {
                Ok((doc, issues)) => {
                    self.upstream_issues.extend(issues.into_iter().map(|w| format!("{}: {w}", readme.display())));
                    if let Some(expected) = &folder_piuri {
                        if &doc.piuri != expected && mappings.piuri.get(&doc.piuri) != Some(&folder) {
                            self.warnings.push(format!(
                                "{}: PIURI {} doesn't match its folder (expected {expected}) and isn't in the mappings file",
                                readme.display(),
                                doc.piuri
                            ));
                        }
                    }
                    if let Some(existing) = self.protocols.get(&doc.piuri) {
                        self.warnings.push(format!(
                            "{}: duplicate PIURI {} (already indexed from {}:{})",
                            readme.display(),
                            doc.piuri,
                            existing.source.name,
                            existing.source.path
                        ));
                        continue;
                    }
                    self.protocols.insert(doc.piuri.clone(), doc);
                }
                Err(e) => self.warnings.push(format!("{}: {e}", readme.display())),
            }
        }
        if source.piuri_base.is_some() {
            for (piuri, path) in &mappings.piuri {
                if piuri.starts_with(source.piuri_base.as_deref().unwrap_or_default()) && !seen_paths.contains(path) {
                    self.warnings.push(format!("mappings: {piuri} = {path:?} names a folder that doesn't exist in {}", source.name));
                }
            }
        }
    }

    fn index_spec(&mut self, source: &SourceConfig, revisions: &HashMap<String, String>) {
        #[derive(Deserialize)]
        struct SpecsJson {
            specs: Vec<SpecEntry>,
        }
        #[derive(Deserialize)]
        struct SpecEntry {
            title: String,
            spec_directory: String,
            markdown_paths: Vec<String>,
        }
        let specs: SpecsJson = match std::fs::read_to_string(source.path.join("specs.json"))
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()))
        {
            Ok(s) => s,
            Err(e) => {
                self.warnings.push(format!("{}/specs.json: {e}", source.path.display()));
                return;
            }
        };
        let revision = revision(source, revisions);
        for spec in specs.specs {
            let version = spec_version(&spec.title);
            let dir = source.path.join(spec.spec_directory.trim_start_matches("./"));
            let mut text = String::new();
            for file in &spec.markdown_paths {
                match std::fs::read_to_string(dir.join(file)) {
                    Ok(t) => {
                        text.push_str(&t);
                        text.push_str("\n\n");
                    }
                    Err(e) => self.warnings.push(format!("{}: {e}", dir.join(file).display())),
                }
            }
            self.specs.insert(
                version.clone(),
                SpecDoc {
                    version,
                    title: spec.title,
                    source: SourceRef {
                        name: source.name.clone(),
                        path: relative(&dir, &source.path),
                        revision: revision.clone(),
                    },
                    sections: markdown::sections(&text),
                },
            );
        }
    }

    /// Every `*.json` under `dir` (recursively), keyed by its `properties.type.const`.
    fn load_schemas(&mut self, dir: &Path, schemas: &mut BTreeMap<String, Value>) {
        for path in walk_json(dir) {
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
            match parsed {
                Ok(schema) => match schema["properties"]["type"]["const"].as_str() {
                    Some(message_type) => {
                        schemas.insert(message_type.to_string(), schema);
                    }
                    None => self.warnings.push(format!("{}: schema has no properties.type.const", path.display())),
                },
                Err(e) => self.warnings.push(format!("{}: {e}", path.display())),
            }
        }
    }

    fn attach_schemas(&mut self, schemas: BTreeMap<String, Value>) {
        for (message_type, schema) in schemas {
            let Some(doc) = message_type.rsplit_once('/').and_then(|(piuri, _)| self.protocols.get_mut(piuri)) else {
                self.warnings.push(format!("schema for {message_type}: no indexed protocol defines it"));
                continue;
            };
            match doc.messages.iter_mut().find(|m| m.message_type == message_type) {
                Some(m) => m.schema = Some(schema),
                None => doc.messages.push(MessageType { message_type, examples: Vec::new(), schema: Some(schema) }),
            }
        }
    }
}

/// Parse one protocol definition. `fallback_piuri` is used when the frontmatter has
/// none. Returns the document and any problems found in its content (see
/// [`Index::upstream_issues`]).
pub fn parse_protocol(
    text: &str,
    source: SourceRef,
    fallback_piuri: Option<&str>,
) -> Result<(ProtocolDoc, Vec<String>), String> {
    let mut warnings = Vec::new();
    let (yaml, body) = markdown::split_frontmatter(text);
    let front: Frontmatter = match yaml {
        Some(yaml) => serde_yaml::from_str(yaml).map_err(|e| format!("frontmatter: {e}"))?,
        None => return Err("no frontmatter".to_string()),
    };
    let piuri = front
        .piuri
        .or_else(|| fallback_piuri.map(str::to_string))
        .ok_or("frontmatter has no piuri")?;
    let sections = markdown::sections(body);
    let roles = sections.iter().find(|s| s.id == "roles").map(|s| roles(&s.markdown)).unwrap_or_default();

    let prefix = format!("{piuri}/");
    let mut messages: Vec<MessageType> = Vec::new();
    for block in markdown::code_blocks(body) {
        let trimmed = block.trim_start();
        if !trimmed.starts_with('{') {
            continue;
        }
        let example: Value = match json5::from_str(&block) {
            Ok(v) => v,
            Err(_) => {
                if block.contains(&prefix) {
                    warnings.push(format!("an example of {piuri} isn't parseable as JSON5"));
                }
                continue;
            }
        };
        let Some(message_type) = example["type"].as_str().or_else(|| example["@type"].as_str()) else {
            continue;
        };
        if !message_type.starts_with(&prefix) {
            continue; // e.g. a discover-features exchange shown inside another protocol
        }
        let message_type = message_type.to_string();
        match messages.iter_mut().find(|m| m.message_type == message_type) {
            Some(m) => m.examples.push(example),
            None => messages.push(MessageType { message_type, examples: vec![example], schema: None }),
        }
    }

    let nonempty = |s: Option<String>| s.filter(|s| !s.trim().is_empty());
    let doc = ProtocolDoc {
        title: nonempty(front.title).unwrap_or_else(|| piuri.clone()),
        status: nonempty(front.status).unwrap_or_else(|| "Unknown".to_string()),
        summary: nonempty(front.summary),
        publisher: nonempty(front.publisher),
        license: nonempty(front.license),
        tags: front.tags.unwrap_or_default(),
        authors: front
            .authors
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.name.as_deref().is_some_and(|n| !n.trim().is_empty()))
            .collect(),
        piuri,
        roles,
        source,
        sections,
        messages,
    };
    Ok((doc, warnings))
}

/// Role names from a "Roles" section. Where roles are introduced as list items
/// (`` - `mediator`: ... ``), only the name leading each item counts, so names merely
/// mentioned in a description (`` receiving `forward` messages ``) don't; otherwise
/// every backticked name does (`` two roles: `sender` and `receiver` ``).
fn roles(section: &str) -> Vec<String> {
    let leading: Vec<String> = section
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix(['-', '*']))
        .map(str::trim_start)
        .filter(|item| item.starts_with('`'))
        .filter_map(|item| markdown::backticked_names(item).into_iter().next())
        .collect();
    if leading.is_empty() {
        markdown::backticked_names(section)
    } else {
        leading
    }
}

/// `2.0` / `2.1` from a spec title ending in `v2.0`; anything else is the editor's draft.
fn spec_version(title: &str) -> String {
    title
        .rsplit_once(" v")
        .map(|(_, v)| v)
        .filter(|v| piuri_version(&format!("x/{v}")).is_some())
        .map(str::to_string)
        .unwrap_or_else(|| "editors-draft".to_string())
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();
    entries
}

fn walk_json(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "json") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// A source's commit: the config's explicit `revision`, else what `git rev-parse` says
/// (the live truth when there's a checkout), else the `index.revisions` file (for the
/// container image, which has neither `.git` nor git).
fn revision(source: &SourceConfig, revisions: &HashMap<String, String>) -> Option<String> {
    if source.revision.is_some() {
        return source.revision.clone();
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(&source.path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|r| !r.is_empty())
        .or_else(|| revisions.get(&source.name).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const README: &str = r#"---
title: Ping Pong
publisher: someone
license: MIT
piuri: https://example.org/ping-pong/1.2
status: Production
summary: Pings.
tags: []
authors:
  - name:
    email:
---

## Roles

There are two roles: `pinger` and `ponger`.

## Message Reference

```json
{
  "id": "1",
  "type": "https://example.org/ping-pong/1.2/ping", // a comment
  "body": {},
}
```

```json
{"type": "https://didcomm.org/discover-features/2.0/disclose", "body": {}}
```

```json
{"@id": "2", "@type": "https://example.org/ping-pong/1.2/ping"}
```
"#;

    fn source() -> SourceRef {
        SourceRef { name: "test".into(), path: "ping-pong/1.2/readme.md".into(), revision: None }
    }

    #[test]
    fn parses_a_protocol_definition() {
        let (doc, issues) = parse_protocol(README, source(), None).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(doc.piuri, "https://example.org/ping-pong/1.2");
        assert_eq!(doc.title, "Ping Pong");
        assert_eq!(doc.roles, ["pinger", "ponger"]);
        assert!(doc.authors.is_empty(), "blank authors are dropped");
        // Only this protocol's examples, v1 and v2 grouped by type; JSON5 accepted.
        assert_eq!(doc.messages.len(), 1);
        assert_eq!(doc.messages[0].message_type, "https://example.org/ping-pong/1.2/ping");
        assert_eq!(doc.messages[0].examples.len(), 2);
        let ids: Vec<_> = doc.sections.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["roles", "message-reference"]);
    }

    #[test]
    fn resolves_message_types_and_minor_versions() {
        let mut index = Index::default();
        let (doc, _) = parse_protocol(README, source(), None).unwrap();
        index.protocols.insert(doc.piuri.clone(), doc);
        let found = |p: &str| index.resolve_protocol(p).map(|d| d.piuri.as_str());

        assert_eq!(found("https://example.org/ping-pong/1.2"), Some("https://example.org/ping-pong/1.2"));
        assert_eq!(found("https://example.org/ping-pong/1.2/ping"), Some("https://example.org/ping-pong/1.2"));
        assert_eq!(found("https://example.org/ping-pong/1.0"), Some("https://example.org/ping-pong/1.2"));
        assert_eq!(found("https://example.org/ping-pong/2.0"), None);
        assert_eq!(found("https://example.org/other/1.2"), None);
    }

    #[test]
    fn roles_from_list_items_or_prose() {
        assert_eq!(roles("- `mediator`: receives `forward` messages.\n- `recipient`: the target."), ["mediator", "recipient"]);
        assert_eq!(roles("There are two roles: `sender` and `receiver`."), ["sender", "receiver"]);
    }

    #[test]
    fn revisions_fall_back_to_the_revisions_file_without_git() {
        let source = |revision: Option<&str>| SourceConfig {
            name: "upstream".into(),
            kind: SourceKind::ProtocolRegistry,
            path: PathBuf::from("/nonexistent/not-a-git-checkout"),
            piuri_base: None,
            revision: revision.map(str::to_string),
        };
        let revisions = HashMap::from([("upstream".to_string(), "abc123".to_string())]);

        assert_eq!(revision(&source(None), &revisions).as_deref(), Some("abc123"));
        assert_eq!(revision(&source(Some("explicit")), &revisions).as_deref(), Some("explicit"));
        assert_eq!(revision(&source(None), &HashMap::new()), None);
    }

    #[test]
    fn spec_versions_come_from_titles() {
        assert_eq!(spec_version("DIDComm Messaging Specification v2.1"), "2.1");
        assert_eq!(spec_version("DIDComm Messaging Specification v2 Editor's Draft"), "editors-draft");
    }
}
