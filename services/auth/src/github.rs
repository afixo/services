//! The GitHub OAuth hop: authorize URL, code exchange, profile fetch.
//! Nothing here touches the database; `server.rs` owns the flow.

use std::time::Duration;

use reqwest::{Client, Url};
use serde::Deserialize;

use crate::settings::GithubConfig;

#[derive(Debug, Clone)]
pub struct Github {
    cfg: GithubConfig,
    http: Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    /// GitHub answered, but said no (bad code, expired code, wrong client).
    #[error("github rejected the request: {0}")]
    Rejected(String),
    #[error("github unreachable: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("github answered something unexpected")]
    Malformed,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

impl Github {
    pub fn new(cfg: GithubConfig) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("afixo-auth")
            .build()
            .expect("reqwest client");
        Self { cfg, http }
    }

    /// Where to send the browser. `state` is the single-use nonce stored in `oauth_states`.
    pub fn authorize_url(&self, state: &str) -> String {
        let mut url = Url::parse(&format!("{}/login/oauth/authorize", self.cfg.oauth_base))
            .expect("oauth base is a url");
        url.query_pairs_mut()
            .append_pair("client_id", &self.cfg.client_id)
            .append_pair("redirect_uri", &self.cfg.redirect_uri)
            .append_pair("scope", "read:user")
            .append_pair("state", state);
        url.into()
    }

    /// Trade the callback `code` for a GitHub access token.
    pub async fn exchange_code(&self, code: &str) -> Result<String, GithubError> {
        let resp: TokenResponse = self
            .http
            .post(format!("{}/login/oauth/access_token", self.cfg.oauth_base))
            .header("accept", "application/json")
            .form(&[
                ("client_id", self.cfg.client_id.as_str()),
                ("client_secret", self.cfg.client_secret.as_str()),
                ("code", code),
                ("redirect_uri", self.cfg.redirect_uri.as_str()),
            ])
            .send()
            .await?
            .json()
            .await
            .map_err(|_| GithubError::Malformed)?;

        match (resp.access_token, resp.error) {
            (Some(token), _) if !token.is_empty() => Ok(token),
            (_, Some(err)) => Err(GithubError::Rejected(
                resp.error_description
                    .map_or(err.clone(), |d| format!("{err}: {d}")),
            )),
            _ => Err(GithubError::Malformed),
        }
    }

    /// The signed-in user's id, login and display name.
    pub async fn fetch_user(&self, access_token: &str) -> Result<GithubUser, GithubError> {
        let resp = self
            .http
            .get(format!("{}/user", self.cfg.api_base))
            .bearer_auth(access_token)
            .header("accept", "application/vnd.github+json")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(GithubError::Rejected(format!(
                "user endpoint answered {}",
                resp.status()
            )));
        }
        let user: GithubUser = resp.json().await.map_err(|_| GithubError::Malformed)?;
        if user.login.is_empty() {
            return Err(GithubError::Malformed);
        }
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GithubConfig {
        GithubConfig {
            client_id: "cid".into(),
            client_secret: "sec".into(),
            redirect_uri: "https://afixo.io/api/v1/auth/github/callback".into(),
            oauth_base: "https://github.com".into(),
            api_base: "https://api.github.com".into(),
        }
    }

    #[test]
    fn authorize_url_carries_state_and_redirect() {
        let url = Github::new(cfg()).authorize_url("st4te");
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("state=st4te"));
        assert!(url.contains("scope=read%3Auser"));
        assert!(
            url.contains(
                "redirect_uri=https%3A%2F%2Fafixo.io%2Fapi%2Fv1%2Fauth%2Fgithub%2Fcallback"
            )
        );
        assert!(
            !url.contains("sec"),
            "the client secret never appears in a browser url"
        );
    }
}
