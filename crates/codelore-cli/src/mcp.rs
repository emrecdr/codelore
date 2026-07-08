//! MCP server for CodeLore (`codelore mcp`).
//!
//! Exposes CodeLore analyses as MCP tools over stdio. All tools are read-only.
//! Each tool call opens its own [`FactsDb`] via the warm-cache path so the
//! `!Send + !Sync` DuckDB connection never crosses thread or await boundaries.

use std::path::PathBuf;

use anyhow::Result;
use rmcp::{handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use codelore_lib::cli_api::{
    Options,
    analyses::{hotspots, summary},
    cache::default_cache_root,
    facts::FactsDb,
    repo::GixRepo,
};

/// Convert an `anyhow::Error` to an MCP `ErrorData` internal error.
fn internal(e: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(e.to_string(), None)
}

/// MCP server state — the repo path fixed at server startup.
#[derive(Clone)]
pub struct CodeLoreServer {
    repo: PathBuf,
}

/// Parameters for the `hotspots` tool.
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct HotspotsParams {
    /// Maximum number of rows to return (default: 20).
    pub limit: Option<u32>,
}

/// Parameters for the `repo_overview` tool (none required).
#[derive(Debug, Deserialize, JsonSchema, Default)]
pub struct RepoOverviewParams {}

#[tool_router(server_handler)]
impl CodeLoreServer {
    /// Return a JSON summary of the repository: commit count, author count,
    /// file count, and date range. First call on a cold cache triggers ingest;
    /// warm-cache calls are fast.
    #[tool(
        name = "repo_overview",
        description = "Return a JSON summary of the repository (commit count, authors, files, date range). First call on a cold cache triggers ingest; warm-cache calls are fast."
    )]
    async fn repo_overview(
        &self,
        _params: Parameters<RepoOverviewParams>,
    ) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        tokio::task::spawn_blocking(move || {
            let opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(internal)?;
            let rows = summary::run_summary(&db, &opts).map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }

    /// Return the top hotspot files by revision count as JSON.
    /// Pass `limit` to cap the number of rows (default: 20).
    /// First call on a cold cache triggers ingest.
    #[tool(
        name = "hotspots",
        description = "Return the top hotspot files by revision count as JSON. Pass `limit` to cap rows (default: 20). First call on a cold cache triggers ingest."
    )]
    async fn hotspots(&self, params: Parameters<HotspotsParams>) -> Result<String, ErrorData> {
        let repo_path = self.repo.clone();
        let limit = params.0.limit.unwrap_or(20);
        tokio::task::spawn_blocking(move || {
            let mut opts = Options {
                repo_path: repo_path.clone(),
                ..Options::default()
            };
            opts.rows_limit = Some(limit);
            let repo = GixRepo::open(&repo_path).map_err(internal)?;
            let db = FactsDb::open_or_ingest_with_cache_root(&opts, &repo, &default_cache_root())
                .map_err(internal)?;
            let rows = hotspots::run_hotspots(&db, &opts).map_err(internal)?;
            serde_json::to_string(&rows).map_err(internal)
        })
        .await
        .map_err(internal)?
    }
}

/// Start the MCP server and block until the client closes the connection.
pub fn run_mcp_server(repo: PathBuf) -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async move {
            let server = CodeLoreServer { repo };
            let transport = rmcp::transport::io::stdio();
            let running = rmcp::service::serve_server(server, transport)
                .await
                .map_err(|e| anyhow::anyhow!("MCP init error: {e}"))?;
            running
                .waiting()
                .await
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("MCP server error: {e}"))
        })
}
