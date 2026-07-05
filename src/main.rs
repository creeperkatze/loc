mod cache;
mod error;
mod inflight;
mod locs;
mod platform;

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tower_http::trace::TraceLayer;

use error::AppError;
use platform::{ForgeClient, Platform};

const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60); // 1 day

#[derive(Clone)]
struct AppState {
    platform_client: Arc<ForgeClient>,
    cache: cache::Cache,
    inflight: inflight::Inflight,
}

#[derive(Serialize)]
struct CacheKey<'a> {
    platform: &'a str,
    owner: &'a str,
    repo: &'a str,
    branch: &'a str,
    filter: &'a Option<String>,
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

    let cache = cache::Cache::open("data/cache.sqlite", CACHE_TTL)
        .await
        .expect("failed to open cache database");

    let state = AppState {
        platform_client: Arc::new(ForgeClient::new()),
        cache,
        inflight: inflight::Inflight::default(),
    };

    let app = Router::new()
        .route("/:platform/:owner/:repo", get(get_locs))
        .route("/:platform/:owner/:repo/badge", get(get_badge))
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

fn parse_platform(platform: &str) -> Result<Platform, AppError> {
    Platform::parse(platform).ok_or_else(|| {
        AppError::NotFound(format!(
            "unsupported platform '{platform}' (expected 'github', 'codeberg', 'gitlab', or 'bitbucket')"
        ))
    })
}

async fn resolve_branch(
    state: &AppState,
    platform: Platform,
    owner: &str,
    repo: &str,
    branch: Option<String>,
) -> Result<String, AppError> {
    match branch {
        Some(branch) => Ok(branch),
        None => {
            state
                .platform_client
                .default_branch(platform, owner, repo)
                .await
        }
    }
}

// Resolves platform/owner/repo/branch/filter to a `Locs` tree, serving from the in-memory TTL cache when possible and populating it on a miss.
async fn get_locs_cached(
    state: &AppState,
    platform: Platform,
    owner: String,
    repo: String,
    branch: Option<String>,
    filter: Option<String>,
) -> Result<Arc<locs::Locs>, AppError> {
    let branch = resolve_branch(state, platform, &owner, &repo, branch).await?;
    let filters = locs::parse_filters(filter.as_deref())?;

    let key = serde_json::to_string(&CacheKey {
        platform: platform.as_str(),
        owner: &owner,
        repo: &repo,
        branch: &branch,
        filter: &filter,
    })
    .expect("serializing cache key");

    if let Some(cached) = state.cache.get(&key).await {
        tracing::debug!(platform = platform.as_str(), %owner, %repo, %branch, "cache hit");
        return Ok(cached);
    }

    // Runs to completion via Inflight even if this request is cancelled; concurrent requests for the same key join it instead of downloading twice.
    let job_state = state.clone();
    let job_key = key.clone();
    let job = async move {
        let tarball = job_state
            .platform_client
            .download_tarball(platform, &owner, &repo, &branch)
            .await?;

        let start = std::time::Instant::now();
        let result = compute_locs_blocking(tarball, filters).await?;

        tracing::info!(platform = platform.as_str(), %owner, %repo, %branch, loc = result.loc, duration_ms = start.elapsed().as_millis(), "computed locs");

        let result = Arc::new(result);
        job_state.cache.insert(job_key, Arc::clone(&result)).await;

        Ok(result)
    };

    state.inflight.run(key, job).await
}

// Decompressing and walking a large tarball is CPU/IO-heavy synchronous work; run it on the blocking thread pool so it doesn't stall the async runtime's worker threads for other in-flight requests.
async fn compute_locs_blocking(
    tarball: Vec<u8>,
    filters: Vec<Regex>,
) -> Result<locs::Locs, AppError> {
    tokio::task::spawn_blocking(move || locs::compute_locs(&tarball, &filters))
        .await
        .map_err(|e| AppError::Upstream(format!("locs computation panicked: {e}")))?
}

async fn get_locs(
    State(state): State<AppState>,
    Path((platform, owner, repo)): Path<(String, String, String)>,
    Query(query): Query<LocsQuery>,
) -> Result<Json<Arc<locs::Locs>>, AppError> {
    let platform = parse_platform(&platform)?;
    let result = get_locs_cached(&state, platform, owner, repo, query.branch, query.filter).await?;
    Ok(Json(result))
}

async fn get_badge(
    State(state): State<AppState>,
    Path((platform, owner, repo)): Path<(String, String, String)>,
    Query(query): Query<BadgeQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let platform = parse_platform(&platform)?;
    let result = get_locs_cached(&state, platform, owner, repo, query.branch, query.filter).await?;

    let message = if query.format.as_deref() == Some("human") {
        locs::humanize(result.loc)
    } else {
        result.loc.to_string()
    };

    Ok(Json(json!({
        "schemaVersion": 1,
        "label": "lines",
        "message": message,
        "cacheSeconds": CACHE_TTL.as_secs(),
    })))
}
