//! The in-memory index of everything the registry serves, built once at startup.

use std::collections::{BTreeMap, BTreeSet, HashMap};
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

/// The prefix DIDComm v1 message types started with, now equivalent to
/// `https://didcomm.org/` (Aries RFC 0348).
pub const LEGACY_PREFIX: &str = "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/";

/// A message type or PIURI with the legacy prefix replaced by `https://didcomm.org/`.
pub fn normalize_type(message_type: &str) -> String {
    match message_type.strip_prefix(LEGACY_PREFIX) {
        Some(rest) => format!("https://didcomm.org/{rest}"),
        None => message_type.to_string(),
    }
}

/// The two ways DIDComm plaintext is shaped: v1 (`@type`, `@id`, `~decorators`,
/// fields at the top level) and v2 (`type`, `id`, headers and a `body`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Envelope {
    V1,
    V2,
}

impl Envelope {
    /// As a `didcomm_versions` entry.
    pub fn range(self) -> &'static str {
        match self {
            Envelope::V1 => "^1.0",
            Envelope::V2 => "^2.0",
        }
    }

    /// The style of an example message, by its type field.
    pub fn of_message(message: &Value) -> Option<Envelope> {
        if message.get("@type").is_some() {
            Some(Envelope::V1)
        } else if message.get("type").is_some() {
            Some(Envelope::V2)
        } else {
            None
        }
    }

    /// The style a schema validates, and the message type it pins: `@type` (by `const`,
    /// or the first `enum` entry) for v1, `type` (by `const`) for v2.
    pub fn of_schema(schema: &Value) -> Option<(Envelope, String)> {
        let properties = &schema["properties"];
        if let Some(t) = properties["type"]["const"].as_str() {
            return Some((Envelope::V2, normalize_type(t)));
        }
        let v1 = &properties["@type"];
        let t = v1["const"].as_str().or_else(|| v1["enum"].as_array()?.iter().find_map(Value::as_str))?;
        Some((Envelope::V1, normalize_type(t)))
    }
}

/// A schema and the envelope style it validates.
#[derive(Debug, Clone)]
pub struct VersionedSchema {
    pub envelope: Envelope,
    pub schema: Value,
}

/// One message type of a protocol, with what the registry knows about it.
#[derive(Debug, Clone, Default)]
pub struct MessageType {
    pub message_type: String,
    pub examples: Vec<Value>,
    /// At most one per envelope style, oldest style first.
    pub schemas: Vec<VersionedSchema>,
}

impl MessageType {
    /// The schema for the newest envelope style (documentation/1.0's `schema`).
    pub fn schema(&self) -> Option<&Value> {
        self.schemas.iter().max_by_key(|s| s.envelope).map(|s| &s.schema)
    }

    /// The schema for one envelope style.
    pub fn schema_for(&self, envelope: Envelope) -> Option<&Value> {
        self.schemas.iter().find(|s| s.envelope == envelope).map(|s| &s.schema)
    }

    /// Envelope styles this type is seen in: its schemas' and its examples'.
    pub fn envelopes(&self) -> BTreeSet<Envelope> {
        self.schemas
            .iter()
            .map(|s| s.envelope)
            .chain(self.examples.iter().filter_map(Envelope::of_message))
            .collect()
    }

    pub fn didcomm_versions(&self) -> Vec<String> {
        self.envelopes().into_iter().map(|e| e.range().to_string()).collect()
    }
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
    /// Other PIURIs this protocol is known by.
    pub aliases: Vec<String>,
    /// `didcomm_versions` as the definition (frontmatter) or its source declares them;
    /// empty = work them out from the message types.
    pub declared_didcomm_versions: Vec<String>,
}

impl ProtocolDoc {
    pub fn has_schemas(&self) -> bool {
        self.messages.iter().any(|m| !m.schemas.is_empty())
    }

    /// Declared, or else every envelope style its message types are seen in. Empty when
    /// neither says (the field is then omitted rather than guessed).
    pub fn didcomm_versions(&self) -> Vec<String> {
        if !self.declared_didcomm_versions.is_empty() {
            return self.declared_didcomm_versions.clone();
        }
        let envelopes: BTreeSet<Envelope> = self.messages.iter().flat_map(MessageType::envelopes).collect();
        envelopes.into_iter().map(|e| e.range().to_string()).collect()
    }

    /// Whether the protocol can be used with DIDComm `version` (`1.0`, `2.1`, ...).
    pub fn supports(&self, version: &semver::Version) -> bool {
        satisfies(&self.didcomm_versions(), version)
    }

    pub fn message(&self, message_type: &str) -> Option<&MessageType> {
        self.messages.iter().find(|m| m.message_type == message_type)
    }
}

/// Whether any of `ranges` (semver requirements) admits `version`.
pub fn satisfies(ranges: &[String], version: &semver::Version) -> bool {
    ranges.iter().filter_map(|r| semver::VersionReq::parse(r).ok()).any(|r| r.matches(version))
}

/// A DIDComm version as written in documentation (`2.1`) as a semver version (`2.1.0`).
pub fn didcomm_version(version: &str) -> Option<semver::Version> {
    let (major, minor) = version.split_once('.')?;
    Some(semver::Version::new(major.parse().ok()?, minor.parse().ok()?, 0))
}

/// One version of a document (a version of the spec, or an extension).
#[derive(Debug, Clone)]
pub struct SpecDoc {
    pub version: String,
    pub title: String,
    pub source: SourceRef,
    pub sections: Vec<Section>,
    pub didcomm_versions: Vec<String>,
}

/// The id of the DIDComm Messaging specification among the documents.
pub const SPEC: &str = "spec";

/// The version of a document that has none of its own.
pub const CURRENT: &str = "current";

/// Something `spec-request` can read: the specification (`spec`), or an extension.
#[derive(Debug, Clone)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub didcomm_versions: Vec<String>,
    pub versions: BTreeMap<String, SpecDoc>,
}

impl Document {
    /// Numbered versions newest first, then the rest (`editors-draft`, `current`).
    pub fn versions_newest_first(&self) -> Vec<&str> {
        let mut numbered: Vec<(semver::Version, &str)> = self
            .versions
            .keys()
            .filter_map(|v| didcomm_version(v).map(|parsed| (parsed, v.as_str())))
            .collect();
        numbered.sort();
        let mut versions: Vec<&str> = numbered.into_iter().rev().map(|(_, v)| v).collect();
        versions.extend(self.versions.keys().map(String::as_str).filter(|v| didcomm_version(v).is_none()));
        versions
    }

    /// `version`, or the newest published (numbered) one, or else the only one.
    pub fn version(&self, version: Option<&str>) -> Option<&SpecDoc> {
        match version {
            Some(v) => self.versions.get(v),
            None => self.versions_newest_first().first().and_then(|v| self.versions.get(*v)),
        }
    }
}

/// Why a `spec-request` couldn't be answered.
#[derive(Debug, PartialEq, Eq)]
pub enum DocumentNotFound {
    Document,
    Version,
}

#[derive(Debug, Default)]
pub struct Index {
    pub protocols: BTreeMap<String, ProtocolDoc>,
    pub documents: BTreeMap<String, Document>,
    /// Alias PIURI → the PIURI it's indexed under.
    pub aliases: BTreeMap<String, String>,
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
    #[serde(default)]
    didcomm_versions: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct Mappings {
    #[serde(default)]
    piuri: HashMap<String, String>,
    /// Alias PIURI → the PIURI a protocol is indexed under.
    #[serde(default)]
    aliases: BTreeMap<String, String>,
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

        index.aliases = mappings.aliases.clone();

        // Overlay schemas first; schemas shipped with a protocol replace them.
        let mut schemas = BTreeMap::new();
        for dir in &config.index.schemas {
            index.load_schemas(dir, true, &mut schemas);
        }
        for source in &config.sources {
            match source.kind {
                SourceKind::ProtocolRegistry => index.index_registry(source, &revisions, &mappings, &mut schemas),
                SourceKind::DidcommSpec => index.index_spec(source, &revisions),
                SourceKind::Rfcs => index.index_rfcs(source, &revisions),
            }
        }
        index.attach_schemas(schemas);
        index.check_aliases();
        index
    }

    /// The protocol a `request` names: an exact PIURI, a message type URI within one,
    /// or the highest minor version under the same major version.
    /// Aliases (including the legacy `did:sov:` prefix) are accepted for both.
    pub fn resolve_protocol(&self, requested: &str) -> Option<&ProtocolDoc> {
        let requested = normalize_type(requested);
        let candidates = [Some(requested.as_str()), requested.rsplit_once('/').map(|(p, _)| p)];
        for piuri in candidates.into_iter().flatten() {
            let piuri = self.aliases.get(piuri).map(String::as_str).unwrap_or(piuri);
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

    /// The document and version a `spec-request` names: `document` defaults to the
    /// spec, `version` to its newest published version.
    pub fn resolve_document(
        &self,
        document: Option<&str>,
        version: Option<&str>,
    ) -> Result<(&Document, &SpecDoc), DocumentNotFound> {
        let document = self.documents.get(document.unwrap_or(SPEC)).ok_or(DocumentNotFound::Document)?;
        let version = document.version(version).ok_or(DocumentNotFound::Version)?;
        Ok((document, version))
    }

    /// The spec version a `spec-request` names; `None` = the newest published version.
    pub fn resolve_spec(&self, version: Option<&str>) -> Option<&SpecDoc> {
        self.resolve_document(None, version).ok().map(|(_, v)| v)
    }

    /// Every alias must name an indexed protocol, and none may shadow one.
    fn check_aliases(&mut self) {
        for (alias, piuri) in &self.aliases {
            if !self.protocols.contains_key(piuri) {
                self.warnings.push(format!("mappings: alias {alias} names {piuri}, which isn't indexed"));
            }
            if self.protocols.contains_key(alias) {
                self.warnings.push(format!("mappings: alias {alias} is itself an indexed protocol"));
            }
        }
        let aliases = self.aliases.clone();
        for doc in self.protocols.values_mut() {
            doc.aliases.extend(aliases.iter().filter(|(_, p)| **p == doc.piuri).map(|(a, _)| a.clone()));
            // The legacy prefix only ever named DIDComm v1 protocols.
            if let Some(rest) = doc.piuri.strip_prefix("https://didcomm.org/") {
                if doc.didcomm_versions().iter().any(|r| r == Envelope::V1.range()) {
                    doc.aliases.push(format!("{LEGACY_PREFIX}{rest}"));
                }
            }
        }
    }

    fn index_registry(
        &mut self,
        source: &SourceConfig,
        revisions: &HashMap<String, String>,
        mappings: &Mappings,
        schemas: &mut BTreeMap<(String, Envelope), Value>,
    ) {
        let revision = revision(source, revisions);
        let mut readmes = Vec::new();
        for name in read_dir_sorted(&source.path) {
            for version in read_dir_sorted(&name) {
                let readme = version.join("readme.md");
                if readme.is_file() {
                    readmes.push(readme);
                }
                self.load_schemas(&version.join("schemas"), false, schemas);
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
            match parse_protocol(&text, source_ref, folder_piuri.as_deref(), &mappings.aliases) {
                Ok((mut doc, issues)) => {
                    if doc.declared_didcomm_versions.is_empty() {
                        doc.declared_didcomm_versions = source.didcomm_versions.clone().unwrap_or_default();
                    }
                    for range in &doc.declared_didcomm_versions {
                        if semver::VersionReq::parse(range).is_err() {
                            self.warnings.push(format!("{}: didcomm_versions entry {range:?} isn't a semver requirement", readme.display()));
                        }
                    }
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
            let didcomm_versions = vec![match didcomm_version(&version) {
                Some(v) => format!("~{}.{}", v.major, v.minor),
                None => Envelope::V2.range().to_string(),
            }];
            let doc = SpecDoc {
                version: version.clone(),
                title: spec.title,
                source: SourceRef {
                    name: source.name.clone(),
                    path: relative(&dir, &source.path),
                    revision: revision.clone(),
                },
                sections: markdown::sections(&text),
                didcomm_versions,
            };
            self.add_document(SPEC, "DIDComm Messaging Specification", &[], doc);
        }
        self.index_extensions(source, revision);
    }

    /// The spec's extensions (`extensions/<name>/main.md`), each the document
    /// `extension/<name>`. They're unversioned (`current`) and apply to DIDComm v2.
    fn index_extensions(&mut self, source: &SourceConfig, revision: Option<String>) {
        let v2 = vec![Envelope::V2.range().to_string()];
        for dir in read_dir_sorted(&source.path.join("extensions")) {
            let main = dir.join("main.md");
            let text = match std::fs::read_to_string(&main) {
                Ok(t) => t,
                Err(e) => {
                    self.warnings.push(format!("{}: {e}", main.display()));
                    continue;
                }
            };
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let sections = markdown::sections(&text);
            let title = sections
                .iter()
                .find(|s| s.level == 1)
                .map(|s| s.title.clone())
                .unwrap_or_else(|| format!("DIDComm {name} extension"));
            let doc = SpecDoc {
                version: CURRENT.to_string(),
                title: title.clone(),
                source: SourceRef { name: source.name.clone(), path: relative(&main, &source.path), revision: revision.clone() },
                // The title's section would repeat the whole document.
                sections: sections.into_iter().filter(|s| s.level > 1).collect(),
                didcomm_versions: v2.clone(),
            };
            self.add_document(&format!("extension/{name}"), &title, &v2, doc);
        }
    }

    /// Add one version of a document, creating the document if it's new.
    pub fn add_document(&mut self, id: &str, title: &str, didcomm_versions: &[String], version: SpecDoc) {
        let document = self.documents.entry(id.to_string()).or_insert_with(|| Document {
            id: id.to_string(),
            title: title.to_string(),
            didcomm_versions: didcomm_versions.to_vec(),
            versions: BTreeMap::new(),
        });
        if document.versions.contains_key(&version.version) {
            self.warnings.push(format!("document {id}: version {} defined twice", version.version));
            return;
        }
        document.versions.insert(version.version.clone(), version);
    }

    /// Every `*.json` under `dir` (recursively), keyed by the message type it pins and
    /// its envelope style. In an `overlay` (laid out `v1/...`, `v2/...`), the folder
    /// must agree with the style.
    fn load_schemas(&mut self, dir: &Path, overlay: bool, schemas: &mut BTreeMap<(String, Envelope), Value>) {
        for path in walk_json(dir) {
            let parsed = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()));
            let schema = match parsed {
                Ok(schema) => schema,
                Err(e) => {
                    self.warnings.push(format!("{}: {e}", path.display()));
                    continue;
                }
            };
            let Some((envelope, message_type)) = Envelope::of_schema(&schema) else {
                self.warnings.push(format!(
                    "{}: schema pins neither properties.type.const (DIDComm v2) nor properties.@type (DIDComm v1)",
                    path.display()
                ));
                continue;
            };
            if overlay {
                let folder = relative(&path, dir).split('/').next().unwrap_or_default().to_string();
                let expected = match envelope {
                    Envelope::V1 => "v1",
                    Envelope::V2 => "v2",
                };
                if folder != expected {
                    self.warnings.push(format!("{}: a DIDComm {expected} schema belongs under {expected}/", path.display()));
                }
            }
            schemas.insert((message_type, envelope), schema);
        }
    }

    fn attach_schemas(&mut self, schemas: BTreeMap<(String, Envelope), Value>) {
        for ((message_type, envelope), schema) in schemas {
            let piuri = message_type.rsplit_once('/').map(|(piuri, _)| piuri).unwrap_or_default();
            let piuri = self.aliases.get(piuri).cloned().unwrap_or_else(|| piuri.to_string());
            let Some(doc) = self.protocols.get_mut(&piuri) else {
                self.warnings.push(format!("schema for {message_type}: no indexed protocol defines it"));
                continue;
            };
            let message_type = format!("{piuri}/{}", message_type.rsplit('/').next().unwrap_or_default());
            let message = match doc.messages.iter_mut().position(|m| m.message_type == message_type) {
                Some(i) => &mut doc.messages[i],
                None => {
                    doc.messages.push(MessageType { message_type, ..MessageType::default() });
                    doc.messages.last_mut().expect("just pushed")
                }
            };
            message.schemas.retain(|s| s.envelope != envelope);
            message.schemas.push(VersionedSchema { envelope, schema });
            message.schemas.sort_by_key(|s| s.envelope);
        }
    }
}

/// Parse one protocol definition. `fallback_piuri` is used when the frontmatter has
/// none. Returns the document and any problems found in its content (see
/// [`Index::upstream_issues`]).
///
/// Examples are grouped by message type, with legacy-prefixed types and those under one
/// of the protocol's `aliases` (alias PIURI → indexed PIURI) filed under the indexed one.
pub fn parse_protocol(
    text: &str,
    source: SourceRef,
    fallback_piuri: Option<&str>,
    aliases: &BTreeMap<String, String>,
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

    let messages = group_examples(&piuri, markdown::code_blocks(body), aliases, &[], &mut warnings);

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
        aliases: Vec::new(),
        declared_didcomm_versions: front.didcomm_versions.unwrap_or_default(),
    };
    Ok((doc, warnings))
}

/// The example messages among `blocks` (code blocks, or whole JSON files) that belong to
/// `piuri`, grouped by message type. A block's type is first rewritten by
/// `substitute` (placeholder → value), then the legacy prefix and `aliases` are
/// resolved; the example itself is kept as written.
pub(crate) fn group_examples(
    piuri: &str,
    blocks: impl IntoIterator<Item = String>,
    aliases: &BTreeMap<String, String>,
    substitute: &[(String, String)],
    warnings: &mut Vec<String>,
) -> Vec<MessageType> {
    let prefix = format!("{piuri}/");
    let mut messages: Vec<MessageType> = Vec::new();
    for block in blocks {
        let trimmed = block.trim_start();
        if !trimmed.starts_with('{') {
            continue;
        }
        let resolve = |written: &str| {
            let mut message_type = written.to_string();
            for (from, to) in substitute {
                message_type = message_type.replace(from.as_str(), to);
            }
            canonical_type(&normalize_type(&message_type), aliases)
        };
        let example: Value = match json5::from_str(&block) {
            Ok(v) => v,
            Err(_) => {
                // Still list the message type, if the broken example names one of ours.
                if let Some(message_type) = written_type(&block).map(resolve).filter(|t| t.starts_with(&prefix)) {
                    warnings.push(format!("an example of {message_type} isn't parseable as JSON5"));
                    if !messages.iter().any(|m| m.message_type == message_type) {
                        messages.push(MessageType { message_type, ..MessageType::default() });
                    }
                } else if block.contains(&prefix) {
                    warnings.push(format!("an example of {piuri} isn't parseable as JSON5"));
                }
                continue;
            }
        };
        let Some(written) = example["type"].as_str().or_else(|| example["@type"].as_str()) else {
            continue;
        };
        let message_type = resolve(written);
        if !message_type.starts_with(&prefix) {
            continue; // e.g. a discover-features exchange shown inside another protocol
        }
        match messages.iter_mut().find(|m| m.message_type == message_type) {
            Some(m) => m.examples.push(example),
            None => messages.push(MessageType { message_type, examples: vec![example], ..MessageType::default() }),
        }
    }
    messages
}

/// The first `"type"` / `"@type"` string value in text that isn't valid JSON5.
fn written_type(block: &str) -> Option<&str> {
    for key in ["\"@type\"", "\"type\""] {
        if let Some(at) = block.find(key) {
            let rest = block[at + key.len()..].trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
            return rest.split_once('"').map(|(value, _)| value);
        }
    }
    None
}

/// A message type with its protocol part replaced when that's an alias.
pub(crate) fn canonical_type(message_type: &str, aliases: &BTreeMap<String, String>) -> String {
    match message_type.rsplit_once('/') {
        Some((piuri, name)) => match aliases.get(piuri) {
            Some(indexed) => format!("{indexed}/{name}"),
            None => message_type.to_string(),
        },
        None => message_type.to_string(),
    }
}

/// Role names from a "Roles" section. Where roles are introduced as list items
/// (`` - `mediator`: ... ``), only the name leading each item counts, so names merely
/// mentioned in a description (`` receiving `forward` messages ``) don't; otherwise
/// every backticked name does (`` two roles: `sender` and `receiver` ``).
pub(crate) fn roles(section: &str) -> Vec<String> {
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

pub(crate) fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
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

pub(crate) fn walk_json(dir: &Path) -> Vec<PathBuf> {
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

pub(crate) fn relative(path: &Path, base: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// A source's commit: the config's explicit `revision`, else what `git rev-parse` says
/// (the live truth when there's a checkout), else the `index.revisions` file (for the
/// container image, which has neither `.git` nor git).
pub(crate) fn revision(source: &SourceConfig, revisions: &HashMap<String, String>) -> Option<String> {
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
        let (doc, issues) = parse_protocol(README, source(), None, &BTreeMap::new()).unwrap();
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
        let (doc, _) = parse_protocol(README, source(), None, &BTreeMap::new()).unwrap();
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
            didcomm_versions: None,
            manifest: None,
        };
        let revisions = HashMap::from([("upstream".to_string(), "abc123".to_string())]);

        assert_eq!(revision(&source(None), &revisions).as_deref(), Some("abc123"));
        assert_eq!(revision(&source(Some("explicit")), &revisions).as_deref(), Some("explicit"));
        assert_eq!(revision(&source(None), &HashMap::new()), None);
    }

    #[test]
    fn envelope_styles_and_versions() {
        let (doc, _) = parse_protocol(README, source(), None, &BTreeMap::new()).unwrap();
        // One v2 and one v1 example of `ping`: both styles, worked out from the examples.
        assert_eq!(doc.messages[0].didcomm_versions(), ["^1.0", "^2.0"]);
        assert_eq!(doc.didcomm_versions(), ["^1.0", "^2.0"]);
        assert!(doc.supports(&didcomm_version("1.0").unwrap()));
        assert!(doc.supports(&didcomm_version("2.1").unwrap()));
        assert!(!doc.supports(&didcomm_version("3.0").unwrap()));

        let declared = README.replace("tags: []", "tags: []\ndidcomm_versions: [\"~2.1\"]");
        let (doc, _) = parse_protocol(&declared, source(), None, &BTreeMap::new()).unwrap();
        assert_eq!(doc.didcomm_versions(), ["~2.1"]);
        assert!(!doc.supports(&didcomm_version("2.0").unwrap()));

        let v2 = serde_json::json!({"properties": {"type": {"const": "https://example.org/p/1.0/m"}}});
        let v1 = serde_json::json!({"properties": {"@type": {"enum": [
            "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/p/1.0/m", "https://didcomm.org/p/1.0/m"
        ]}}});
        assert_eq!(Envelope::of_schema(&v2), Some((Envelope::V2, "https://example.org/p/1.0/m".into())));
        assert_eq!(Envelope::of_schema(&v1), Some((Envelope::V1, "https://didcomm.org/p/1.0/m".into())));
        assert_eq!(Envelope::of_schema(&serde_json::json!({})), None);
    }

    #[test]
    fn legacy_prefixes_and_aliases_are_filed_under_the_indexed_piuri() {
        let readme = r#"---
title: Trust Ping
piuri: https://didcomm.org/trust_ping/1.0
status: Adopted
---
```json
{"@type": "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/trust_ping/1.0/ping", "@id": "1"}
```
```json
{"@type": "https://didcomm.org/trust-ping/1.0/ping", "@id": "2"}
```
"#;
        let aliases = BTreeMap::from([(
            "https://didcomm.org/trust-ping/1.0".to_string(),
            "https://didcomm.org/trust_ping/1.0".to_string(),
        )]);
        let (doc, _) = parse_protocol(readme, source(), None, &aliases).unwrap();
        assert_eq!(doc.messages.len(), 1);
        assert_eq!(doc.messages[0].message_type, "https://didcomm.org/trust_ping/1.0/ping");
        assert_eq!(doc.messages[0].examples.len(), 2);

        let mut index = Index { aliases, ..Index::default() };
        index.protocols.insert(doc.piuri.clone(), doc);
        index.check_aliases();
        let found = |p: &str| index.resolve_protocol(p).map(|d| d.piuri.as_str());
        assert_eq!(found("https://didcomm.org/trust-ping/1.0/ping"), Some("https://didcomm.org/trust_ping/1.0"));
        assert_eq!(found("did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/trust_ping/1.0"), Some("https://didcomm.org/trust_ping/1.0"));
        assert_eq!(
            index.protocols["https://didcomm.org/trust_ping/1.0"].aliases,
            ["https://didcomm.org/trust-ping/1.0", "did:sov:BzCbsNYhMrjHiqZDTUASHg;spec/trust_ping/1.0"]
        );
        assert!(index.warnings.is_empty(), "{:?}", index.warnings);
    }

    #[test]
    fn document_versions_newest_first() {
        let version = |v: &str| SpecDoc {
            version: v.into(),
            title: v.into(),
            source: source(),
            sections: Vec::new(),
            didcomm_versions: Vec::new(),
        };
        let mut index = Index::default();
        for v in ["2.0", "editors-draft", "1.0", "2.1", "10.0"] {
            index.add_document(SPEC, "Spec", &[], version(v));
        }
        assert_eq!(index.documents[SPEC].versions_newest_first(), ["10.0", "2.1", "2.0", "1.0", "editors-draft"]);
        assert_eq!(index.resolve_spec(None).unwrap().version, "10.0");
        assert_eq!(index.resolve_document(Some("extension/x"), None).unwrap_err(), DocumentNotFound::Document);
        assert_eq!(index.resolve_document(None, Some("9.9")).unwrap_err(), DocumentNotFound::Version);

        index.add_document("extension/x", "X", &["^2.0".into()], version(CURRENT));
        assert_eq!(index.resolve_document(Some("extension/x"), None).unwrap().1.version, CURRENT);
    }

    #[test]
    fn broken_examples_still_name_their_type() {
        let blocks = vec!["{\n  \"@type\": \"<base>/hello\",\n  \"a\": 1 // no comma\n  \"b\": 2\n}".to_string()];
        let substitute = [("<base>".to_string(), "https://example.org/p/1.0".to_string())];
        let mut warnings = Vec::new();
        let messages = group_examples("https://example.org/p/1.0", blocks, &BTreeMap::new(), &substitute, &mut warnings);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, "https://example.org/p/1.0/hello");
        assert!(messages[0].examples.is_empty());
        assert_eq!(warnings, ["an example of https://example.org/p/1.0/hello isn't parseable as JSON5"]);
    }

    #[test]
    fn spec_versions_come_from_titles() {
        assert_eq!(spec_version("DIDComm Messaging Specification v2.1"), "2.1");
        assert_eq!(spec_version("DIDComm Messaging Specification v2 Editor's Draft"), "editors-draft");
    }
}
