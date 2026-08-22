//! auth-service configuration and the token lifetimes (the contract in
//! `docs/domain.md`: access 15 min, refresh 7 d, requester token 1 h, OAuth
//! state 10 min).

use afixo_common::config::{self, ConfigError};
use time::Duration;

pub const ACCESS_TTL: Duration = Duration::minutes(15);
pub const REFRESH_TTL: Duration = Duration::days(7);
pub const CLIENT_TOKEN_TTL: Duration = Duration::hours(1);
pub const STATE_TTL: Duration = Duration::minutes(10);

/// GitHub OAuth app settings. `oauth_base`/`api_base` exist so tests can point
/// at a local stand-in; production leaves them at the defaults.
#[derive(Debug, Clone)]
pub struct GithubConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub oauth_base: String,
    pub api_base: String,
}

impl GithubConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            client_id: config::required("GITHUB_CLIENT_ID")?,
            client_secret: config::required("GITHUB_CLIENT_SECRET")?,
            redirect_uri: config::required("GITHUB_REDIRECT_URI")?,
            oauth_base: config::or("GITHUB_OAUTH_BASE", "https://github.com"),
            api_base: config::or("GITHUB_API_BASE", "https://api.github.com"),
        })
    }
}
