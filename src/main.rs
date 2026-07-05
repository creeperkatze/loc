mod error;
mod github;
mod locs;

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use regex::Regex;
use serde::Deserialize;
use serde_json::json;
use tower_http::trace::TraceLayer;

use error::AppError;
use github::GitHubClient;

#[derive(Clone)]
struct AppState {
    github: Arc<GitHubClient>,
}

#[derive(Deserialize)]
struct LocsQuery {
    branch: Option<String>,
    filter: Option<String>,
}

#[derive(Deserialize)]
struct BadgeQuery {
    branch: Option<String>,
    filter: Option<String>,
    format: Option<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let state = AppState {
        github: Arc::new(GitHubClient::new()),
    };

    let app = Router::new()
        .route("/:owner/:repo/locs", get(get_locs))
        .route("/:owner/:repo/badge", get(get_badge))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("failed to bind port");

    tracing::info!("listening on http://0.0.0.0:{port}");

    axum::serve(listener, app).await.expect("server error");
}

async fn resolve_branch(
    state: &AppState,
    owner: &str,
    repo: &str,
    branch: Option<String>,
) -> Result<String, AppError> {
    match branch {
        Some(branch) => Ok(branch),
        None => state.github.default_branch(owner, repo).await,
    }
}

/// Decompressing and walking a large tarball is CPU/IO-heavy synchronous
/// work; run it on the blocking thread pool so it doesn't stall the async
/// runtime's worker threads for other in-flight requests.
async fn compute_locs_blocking(tarball: Vec<u8>, filters: Vec<Regex>) -> Result<locs::Locs, AppError> {
    tokio::task::spawn_blocking(move || locs::compute_locs(&tarball, &filters))
        .await
        .map_err(|e| AppError::Upstream(format!("locs computation panicked: {e}")))?
}

async fn get_locs(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<LocsQuery>,
) -> Result<Json<locs::Locs>, AppError> {
    let branch = resolve_branch(&state, &owner, &repo, query.branch).await?;
    let filters = locs::parse_filters(query.filter.as_deref())?;

    let tarball = state.github.download_tarball(&owner, &repo, &branch).await?;
    let result = compute_locs_blocking(tarball, filters).await?;

    tracing::info!(%owner, %repo, %branch, loc = result.loc, "computed locs");
    Ok(Json(result))
}

async fn get_badge(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<BadgeQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let branch = resolve_branch(&state, &owner, &repo, query.branch).await?;
    let filters = locs::parse_filters(query.filter.as_deref())?;

    let tarball = state.github.download_tarball(&owner, &repo, &branch).await?;
    let result = compute_locs_blocking(tarball, filters).await?;

    tracing::info!(%owner, %repo, %branch, loc = result.loc, "computed locs for badge");

    let message = if query.format.as_deref() == Some("human") {
        locs::humanize(result.loc)
    } else {
        result.loc.to_string()
    };

    Ok(Json(json!({
        "schemaVersion": 1,
        "label": "lines",
        "message": message,
        "cacheSeconds": 15 * 60,
    })))
}
