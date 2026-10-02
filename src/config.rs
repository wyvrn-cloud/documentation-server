//! `config/default.toml` and friends.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub identity: IdentityConfig,
    #[serde(default)]
    pub index: IndexConfig,
    #[serde(default, rename = "source")]
    pub sources: Vec<SourceConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub bind: String,
    pub public_url: String,
    #[serde(default)]
    pub did_method: DidMethod,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DidMethod {
    #[default]
    Peer,
    Web,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexConfig {
    #[serde(default)]
    pub schemas: Vec<PathBuf>,
    pub mappings: Option<PathBuf>,
    /// Source name → commit, for when `git rev-parse` can't tell (the container image
    /// has no `.git`). See `sources/revisions.toml`.
    pub revisions: Option<PathBuf>,
    #[serde(default = "default_max_query_limit")]
    pub max_query_limit: usize,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self { schemas: Vec::new(), mappings: None, revisions: None, max_query_limit: default_max_query_limit() }
    }
}

fn default_max_query_limit() -> usize {
    200
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    pub name: String,
    pub kind: SourceKind,
    pub path: PathBuf,
    /// For `protocol-registry`: the PIURI prefix a folder `<name>/<version>` is expected
    /// to have, for the folder/PIURI consistency check.
    pub piuri_base: Option<String>,
    /// Overrides the source revision otherwise read with `git rev-parse`, or else from
    /// `index.revisions` (e.g. in a
    /// container image built without `.git`).
    pub revision: Option<String>,
    /// `didcomm_versions` for every protocol in this source whose definition doesn't
    /// declare its own (e.g. `["^1.0"]` for the Aries RFCs). Unset = work them out from
    /// each protocol's examples and schemas.
    pub didcomm_versions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Folders of `<name>/<version>/readme.md` in didcomm.org's format.
    ProtocolRegistry,
    /// A checkout of decentralized-identity/didcomm-messaging (reads `specs.json`).
    DidcommSpec,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("reading config {}: {e}", path.display()))?;
        Ok(toml::from_str(&text)?)
    }
}
