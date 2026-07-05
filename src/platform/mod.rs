mod bitbucket;
mod codeberg;
mod github;
mod gitlab;

use std::time::Instant;

use crate::error::AppError;

// A git hosting platform we know how to talk to. Each variant's URL building, auth, and response parsing lives in its own submodule.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum Platform {
    GitHub,
    Codeberg,
    GitLab,
    Bitbucket,
}

impl Platform {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Platform::GitHub),
            "codeberg" => Some(Platform::Codeberg),
            "gitlab" => Some(Platform::GitLab),
            "bitbucket" => Some(Platform::Bitbucket),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Platform::GitHub => "github",
            Platform::Codeberg => "codeberg",
            Platform::GitLab => "gitlab",
            Platform::Bitbucket => "bitbucket",
        }
    }
}

pub struct ForgeClient {
    client: reqwest::Client,
}

impl ForgeClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("creeperkatze/loctopus/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build http client");

        Self { client }
    }

    // Fetches a repo's default branch.
    pub async fn default_branch(
        &self,
        platform: Platform,
        owner: &str,
        repo: &str,
    ) -> Result<String, AppError> {
        tracing::debug!(platform = platform.as_str(), %owner, %repo, "fetching default branch");
        let start = Instant::now();

        let branch = match platform {
            Platform::GitHub => github::default_branch(&self.client, owner, repo).await,
            Platform::Codeberg => codeberg::default_branch(&self.client, owner, repo).await,
            Platform::GitLab => gitlab::default_branch(&self.client, owner, repo).await,
            Platform::Bitbucket => bitbucket::default_branch(&self.client, owner, repo).await,
        }?;

        tracing::debug!(platform = platform.as_str(), %owner, %repo, %branch, duration_ms = start.elapsed().as_millis(), "resolved default branch");
        Ok(branch)
    }

    // Opens a repo's source tree at a given branch as a gzipped tarball stream; the caller decodes it as bytes arrive instead of buffering the whole download first.
    pub async fn download_tarball(
        &self,
        platform: Platform,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<reqwest::Response, AppError> {
        tracing::debug!(platform = platform.as_str(), %owner, %repo, %branch, "opening tarball stream");

        match platform {
            Platform::GitHub => github::download_tarball(&self.client, owner, repo, branch).await,
            Platform::Codeberg => codeberg::download_tarball(&self.client, owner, repo, branch).await,
            Platform::GitLab => gitlab::download_tarball(&self.client, owner, repo, branch).await,
            Platform::Bitbucket => bitbucket::download_tarball(&self.client, owner, repo, branch).await,
        }
    }
}

// Shared request-execution helpers used by every platform submodule: run the request, map 404 to `AppError::NotFound`, anything else non-2xx to `AppError::Upstream`.
async fn fetch_json(
    request: reqwest::RequestBuilder,
    not_found: impl FnOnce() -> String,
) -> Result<serde_json::Value, AppError> {
    let response = request
        .send()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(not_found()));
    }
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "upstream API returned {}",
            response.status()
        )));
    }

    response
        .json()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))
}

// Like `fetch_json`, but hands back the still-streaming response instead of buffering the body, so the caller can decode it as it arrives.
async fn fetch_stream(
    request: reqwest::RequestBuilder,
    not_found: impl FnOnce() -> String,
) -> Result<reqwest::Response, AppError> {
    let response = request
        .send()
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(AppError::NotFound(not_found()));
    }
    if !response.status().is_success() {
        return Err(AppError::Upstream(format!(
            "failed to download source: {}",
            response.status()
        )));
    }

    Ok(response)
}
