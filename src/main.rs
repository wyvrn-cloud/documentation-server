use std::sync::Arc;

use anyhow::Context;
use didcomm_agent::Identity;
use documentation_server::{
    config::Config,
    index::Index,
    registry::Registry,
    server::{self, AppState},
};

/// Config path: the first argument, else `$DOCSERVER_CONFIG`, else
/// `config/default.toml`.
fn config_path() -> String {
    std::env::args()
        .nth(1)
        .or_else(|| std::env::var("DOCSERVER_CONFIG").ok())
        .unwrap_or_else(|| "config/default.toml".to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::load(config_path())?;
    let index = Index::build(&config);
    for warning in &index.warnings {
        tracing::warn!("{warning}");
    }
    for issue in &index.upstream_issues {
        tracing::info!("upstream content: {issue}");
    }
    tracing::info!(
        protocols = index.protocols.len(),
        specs = ?index.specs.keys().collect::<Vec<_>>(),
        warnings = index.warnings.len(),
        "index built"
    );

    let identity = Identity::load_or_generate(&config.identity.path)
        .with_context(|| format!("identity file {}", config.identity.path.display()))?;
    let (agent, did_document) = server::build_agent(&config, identity)?;
    tracing::info!(did = %agent.did(), endpoint = %config.server.public_url, "registry ready");

    let state = Arc::new(AppState {
        agent,
        registry: Registry { index, max_query_limit: config.index.max_query_limit },
        did_document,
    });
    let listener = tokio::net::TcpListener::bind(&config.server.bind)
        .await
        .with_context(|| format!("binding {}", config.server.bind))?;
    axum::serve(listener, server::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
