use crate::error::AppError;

pub struct GitHubClient {
    client: reqwest::Client,
    token: Option<String>,
}

impl GitHubClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent("loc")
            .build()
            .expect("failed to build http client");

        Self {
            client,
            token: std::env::var("GITHUB_TOKEN").ok(),
        }
    }

    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        let request = self.client.get(url);
        match &self.token {
            Some(token) => request.header("Authorization", format!("token {token}")),
            None => request,
        }
    }

    /// Fetches a repo's default branch from the GitHub API.
    pub async fn default_branch(&self, owner: &str, repo: &str) -> Result<String, AppError> {
        let url = format!("https://api.github.com/repos/{owner}/{repo}");
        tracing::debug!(%owner, %repo, "fetching default branch");

        let response = self
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::NotFound(format!(
                "repository {owner}/{repo} not found"
            )));
        }
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "GitHub API returned {}",
                response.status()
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        let branch = body
            .get("default_branch")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| AppError::Upstream("missing default_branch in GitHub response".into()))?;

        tracing::debug!(%owner, %repo, %branch, "resolved default branch");
        Ok(branch)
    }

    /// Downloads a repo's source tree at a given branch as a gzipped tarball.
    pub async fn download_tarball(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<Vec<u8>, AppError> {
        let url = format!("https://codeload.github.com/{owner}/{repo}/tar.gz/refs/heads/{branch}");
        tracing::info!(%owner, %repo, %branch, "downloading tarball");

        let response = self
            .get(&url)
            .send()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::NotFound(format!(
                "branch '{branch}' not found in {owner}/{repo}"
            )));
        }
        if !response.status().is_success() {
            return Err(AppError::Upstream(format!(
                "failed to download source: {}",
                response.status()
            )));
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| AppError::Upstream(e.to_string()))?;

        tracing::info!(%owner, %repo, %branch, bytes = bytes.len(), "downloaded tarball");
        Ok(bytes.to_vec())
    }
}
