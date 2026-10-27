use super::error::UpdateError;
use reqwest::Client;
use reqwest::header::{ACCEPT, USER_AGENT};
use std::fs;
use std::path::Path;
use std::time::Duration;

const GITHUB_REPO: &str = "possible055/swe-grep";
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 10;
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MAX_DOWNLOAD_RETRIES: usize = 3;

#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub archive_url: String,
    pub checksum_url: String,
    pub archive_name: String,
}

pub struct UpdateClient {
    client: Client,
    base_url_override: Option<String>,
}

impl UpdateClient {
    pub fn new(base_url_override: Option<String>) -> Result<Self, UpdateError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()?;
        Ok(Self {
            client,
            base_url_override,
        })
    }

    pub fn target_archive_name() -> Result<&'static str, UpdateError> {
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Ok("swe-grep-linux-x86_64-musl.tar.gz")
        } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            Ok("swe-grep-windows-x86_64.zip")
        } else {
            Err(UpdateError::NoAssetForPlatform(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            )))
        }
    }

    pub async fn fetch_latest_release(
        &self,
        target_tag: Option<&str>,
    ) -> Result<ReleaseInfo, UpdateError> {
        let archive_name = Self::target_archive_name()?;

        if let Some(base_url) = &self.base_url_override {
            let tag = target_tag
                .filter(|tag| !tag.trim().is_empty())
                .ok_or(UpdateError::MissingMirrorTag)?;
            let base = base_url.trim_end_matches('/');
            return Ok(ReleaseInfo {
                tag_name: tag.to_string(),
                archive_url: format!("{base}/{archive_name}"),
                checksum_url: format!("{base}/{archive_name}.sha256"),
                archive_name: archive_name.to_string(),
            });
        }

        let tag_name = if let Some(tag) = target_tag {
            tag.to_string()
        } else {
            self.discover_latest_tag().await?
        };

        let base = format!("https://github.com/{GITHUB_REPO}/releases/download/{tag_name}");
        Ok(ReleaseInfo {
            archive_url: format!("{base}/{archive_name}"),
            checksum_url: format!("{base}/{archive_name}.sha256"),
            tag_name,
            archive_name: archive_name.to_string(),
        })
    }

    async fn discover_latest_tag(&self) -> Result<String, UpdateError> {
        let user_agent = format!("swe-grep/{}", env!("CARGO_PKG_VERSION"));
        let api_url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");

        let api_response = self
            .client
            .get(&api_url)
            .header(USER_AGENT, &user_agent)
            .header(ACCEPT, "application/vnd.github.v3+json")
            .send()
            .await;

        if let Ok(res) = api_response
            && res.status().is_success()
            && let Ok(bytes) = res.bytes().await
        {
            let maybe_tag = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|json| json.get("tag_name")?.as_str().map(ToString::to_string));
            if let Some(tag) = maybe_tag {
                return Ok(tag);
            }
        }

        // Fallback: HEAD request to releases/latest to extract redirected tag from URL
        let redirect_url = format!("https://github.com/{GITHUB_REPO}/releases/latest");
        let head_res = self
            .client
            .head(&redirect_url)
            .header(USER_AGENT, &user_agent)
            .send()
            .await?
            .error_for_status()?;

        release_tag_from_url(head_res.url().as_str())
    }

    pub async fn download_with_retry(
        &self,
        url: &str,
        destination: &Path,
    ) -> Result<(), UpdateError> {
        let user_agent = format!("swe-grep/{}", env!("CARGO_PKG_VERSION"));
        let mut last_error = None;

        for attempt in 1..=MAX_DOWNLOAD_RETRIES {
            let response = self
                .client
                .get(url)
                .header(USER_AGENT, &user_agent)
                .send()
                .await;

            match response {
                Ok(res) if res.status().is_success() => {
                    let bytes = res.bytes().await?;
                    fs::write(destination, bytes)?;
                    return Ok(());
                }
                Ok(res) => {
                    last_error = Some(UpdateError::Extraction(format!(
                        "HTTP request failed with status: {} on {}",
                        res.status(),
                        url
                    )));
                }
                Err(err) => {
                    last_error = Some(UpdateError::Network(err));
                }
            }

            if attempt < MAX_DOWNLOAD_RETRIES {
                tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
            }
        }

        Err(last_error.unwrap_or_else(|| {
            UpdateError::Extraction(format!(
                "Download failed after {MAX_DOWNLOAD_RETRIES} attempts"
            ))
        }))
    }
}

fn release_tag_from_url(url: &str) -> Result<String, UpdateError> {
    if let Some(tag) = url.split("/releases/tag/").nth(1) {
        let clean_tag = tag.split(&['/', '?', '#'][..]).next().unwrap_or(tag);
        if !clean_tag.is_empty() {
            return Ok(clean_tag.to_string());
        }
    }
    Err(UpdateError::ReleaseDiscovery(url.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mirror_requires_an_explicit_release_tag() {
        let client = UpdateClient::new(Some("https://mirror.example/v0.1.11/".into())).unwrap();
        let error = client.fetch_latest_release(None).await.unwrap_err();
        assert!(matches!(error, UpdateError::MissingMirrorTag));

        let release = client.fetch_latest_release(Some("v0.1.11")).await.unwrap();
        assert_eq!(release.tag_name, "v0.1.11");
        assert_eq!(
            release.archive_url,
            format!("https://mirror.example/v0.1.11/{}", release.archive_name)
        );
    }

    #[test]
    fn release_redirect_requires_a_nonempty_tag() {
        let url = "https://github.com/possible055/swe-grep/releases/tag/v0.1.11?x=1#notes";
        assert_eq!(release_tag_from_url(url).unwrap(), "v0.1.11");
        for url in [
            "https://github.com/possible055/swe-grep/releases/latest",
            "https://github.com/possible055/swe-grep/releases/tag/",
        ] {
            assert!(matches!(
                release_tag_from_url(url).unwrap_err(),
                UpdateError::ReleaseDiscovery(failed_url) if failed_url == url
            ));
        }
    }
}
