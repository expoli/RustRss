//! Shared outbound proxy configuration. Loopback MCP transport is configured separately.
use crate::{Store, StoreError};
use serde::{Deserialize, Serialize};

pub const PROXY_KEY: &str = "network.proxy";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Environment,
    Direct,
    Custom,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    pub mode: ProxyMode,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub no_proxy: String,
}

impl ProxyConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.mode != ProxyMode::Custom && self.url.is_empty() {
            return Ok(());
        }
        let url = url::Url::parse(&self.url).map_err(|_| "proxy_invalid_url")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("proxy_invalid_url".into());
        }
        // Credentials must never be persisted in the settings database.
        if !url.username().is_empty() || url.password().is_some() {
            return Err("proxy_credentials_not_supported".into());
        }
        Ok(())
    }

    pub fn load(store: &Store) -> Result<Self, StoreError> {
        let value = match store.setting(PROXY_KEY)? {
            Some(value) => serde_json::from_str::<Self>(&value)
                .map_err(|_| StoreError::Invalid("proxy_invalid_config".into()))?,
            None => Self::default(),
        };
        value.validate().map_err(StoreError::Invalid)?;
        Ok(value)
    }

    pub fn save(&self, store: &Store) -> Result<(), StoreError> {
        self.validate().map_err(StoreError::Invalid)?;
        store.set_setting(PROXY_KEY, &serde_json::to_string(self).unwrap())
    }

    pub fn apply(&self, builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, String> {
        self.validate()?;
        Ok(match self.mode {
            ProxyMode::Environment => builder,
            ProxyMode::Direct => builder.no_proxy(),
            ProxyMode::Custom => builder.no_proxy().proxy(
                reqwest::Proxy::all(&self.url)
                    .map_err(|_| "proxy_invalid_url")?
                    .no_proxy(reqwest::NoProxy::from_string(&self.no_proxy)),
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_roundtrip_and_invalid_input_does_not_overwrite() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(ProxyConfig::load(&store).unwrap(), ProxyConfig::default());
        let mut value = ProxyConfig {
            mode: ProxyMode::Custom,
            url: "http://127.0.0.1:8080".into(),
            no_proxy: "localhost,127.0.0.1".into(),
        };
        value.save(&store).unwrap();
        assert_eq!(ProxyConfig::load(&store).unwrap(), value);
        let saved = store.setting(PROXY_KEY).unwrap();
        for invalid in [
            "bad",
            "file:///tmp/socket",
            "http://user:secret@localhost:8080",
            "http://localhost/path",
            "http://localhost?password=secret",
        ] {
            value.url = invalid.into();
            assert!(value.save(&store).is_err());
            assert_eq!(store.setting(PROXY_KEY).unwrap(), saved);
        }
    }
}
