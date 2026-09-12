use std::{
    env, fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not read configuration {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid configuration: {0}")]
    Parse(#[from] serde_yaml::Error),
    #[error("invalid configuration: {0}")]
    Validation(String),
    #[error("invalid environment variable {name}: {message}")]
    Environment { name: &'static str, message: String },
}

/// Complete runtime configuration. Unknown YAML keys are rejected so typos do
/// not silently weaken a deployment.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub redirects: RedirectConfig,
    pub catalog: CatalogConfig,
    pub storage: StorageConfig,
    pub security: SecurityConfig,
    pub dns: DnsConfig,
}

impl Config {
    pub fn from_yaml(yaml: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(yaml)?;
        config.validate()?;
        Ok(config)
    }

    /// Loads YAML and then applies the documented `WAYBEND_*` environment
    /// overrides. Passing `None` creates a production-safe default config.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut config = match path {
            Some(path) => {
                let yaml = fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                })?;
                serde_yaml::from_str(&yaml)?
            }
            None => Self::default(),
        };
        config.apply_environment()?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server.body_limit == 0 {
            return Err(invalid("server.body_limit must be greater than zero"));
        }
        if self.server.cors_origins.iter().any(|origin| origin == "*")
            && self.server.cors_origins.len() != 1
        {
            return Err(invalid(
                "server.cors_origins wildcard must not be combined with other origins",
            ));
        }
        for origin in self
            .server
            .cors_origins
            .iter()
            .filter(|origin| origin.as_str() != "*")
        {
            let parsed = Url::parse(origin).map_err(|error| {
                invalid(format!(
                    "server.cors_origins contains an invalid URL: {error}"
                ))
            })?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err(invalid(
                    "server.cors_origins entries must be absolute HTTP origins without paths, credentials, queries, or fragments",
                ));
            }
        }
        if let Some(url) = &self.server.public_url {
            let parsed = Url::parse(url)
                .map_err(|error| invalid(format!("server.public_url is not a URL: {error}")))?;
            if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                return Err(invalid(
                    "server.public_url must be an absolute HTTP or HTTPS URL",
                ));
            }
            if parsed.query().is_some() || parsed.fragment().is_some() {
                return Err(invalid(
                    "server.public_url must not contain a query string or fragment",
                ));
            }
            if parsed.path() != "/" {
                return Err(invalid("server.public_url must not contain a path"));
            }
        }
        if !matches!(self.redirects.default_status, 301 | 302 | 303 | 307 | 308) {
            return Err(invalid(
                "redirects.default_status must be one of 301, 302, 303, 307, or 308",
            ));
        }
        if self.redirects.default_hops == 0
            || self.redirects.max_hops == 0
            || self.redirects.default_hops > self.redirects.max_hops
        {
            return Err(invalid(
                "redirect hop counts must be positive and default_hops must not exceed max_hops",
            ));
        }
        if self
            .catalog
            .required_tags
            .iter()
            .any(|tag| tag.trim().is_empty())
            || self
                .catalog
                .enabled_categories
                .iter()
                .any(|category| category.trim().is_empty())
        {
            return Err(invalid("catalog filters must not contain empty values"));
        }
        self.dns.validate()?;
        Ok(())
    }

    fn apply_environment(&mut self) -> Result<(), ConfigError> {
        if let Some(value) = env_value("WAYBEND_LISTEN") {
            self.server.listen = parse_env("WAYBEND_LISTEN", &value)?;
        }
        if let Some(value) = env_value("WAYBEND_PUBLIC_URL") {
            self.server.public_url = nonempty_or_none(value);
        }
        if let Some(value) = env_value("WAYBEND_DATA_DIR")
            && env_value("WAYBEND_DATABASE").is_none()
        {
            self.storage.database = PathBuf::from(value).join("waybend.sqlite3");
        }
        if let Some(value) = env_value("WAYBEND_DATABASE") {
            self.storage.database = PathBuf::from(value);
        }
        if let Some(value) = env_value("WAYBEND_DEFAULT_STATUS") {
            self.redirects.default_status = parse_env("WAYBEND_DEFAULT_STATUS", &value)?;
        }
        if let Some(value) = env_value("WAYBEND_DEFAULT_HOPS") {
            self.redirects.default_hops = parse_env("WAYBEND_DEFAULT_HOPS", &value)?;
        }
        if let Some(value) = env_value("WAYBEND_MAX_HOPS") {
            self.redirects.max_hops = parse_env("WAYBEND_MAX_HOPS", &value)?;
        }
        if let Some(value) = env_value("WAYBEND_PACKS") {
            self.catalog.external_packs = env::split_paths(&value).collect();
        }
        if let Some(value) = env_value("WAYBEND_DNS_LISTEN") {
            self.dns.udp_listen = parse_env("WAYBEND_DNS_LISTEN", &value)?;
            self.dns.tcp_listen = self.dns.udp_listen;
        }
        if let Some(value) = env_value("WAYBEND_REBIND_DOMAIN") {
            self.dns.domain = nonempty_or_none(value);
            self.dns.enabled = self.dns.domain.is_some();
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    pub public_url: Option<String>,
    pub trust_proxy: bool,
    pub body_limit: usize,
    pub cors_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".parse().expect("static socket address"),
            public_url: None,
            trust_proxy: false,
            body_limit: 1024 * 1024,
            cors_origins: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RedirectConfig {
    pub default_status: u16,
    pub default_hops: u8,
    pub max_hops: u8,
    pub preserve_query: bool,
    pub allow_target_override: bool,
}

impl Default for RedirectConfig {
    fn default() -> Self {
        Self {
            default_status: 302,
            default_hops: 3,
            max_hops: 10,
            preserve_query: false,
            allow_target_override: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct CatalogConfig {
    pub include_builtin: bool,
    pub external_packs: Vec<PathBuf>,
    pub enabled_categories: Vec<String>,
    pub required_tags: Vec<String>,
    pub disabled_ids: Vec<String>,
}

impl Default for CatalogConfig {
    fn default() -> Self {
        Self {
            include_builtin: true,
            external_packs: Vec::new(),
            enabled_categories: Vec::new(),
            required_tags: Vec::new(),
            disabled_ids: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub database: PathBuf,
    pub retention_days: u32,
    pub max_body_bytes: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub admin_token: Option<String>,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database: PathBuf::from("./data/waybend.sqlite3"),
            retention_days: 30,
            max_body_bytes: 256 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct DnsConfig {
    pub enabled: bool,
    pub udp_listen: SocketAddr,
    pub tcp_listen: SocketAddr,
    pub domain: Option<String>,
    pub ttl: u32,
    pub first_ip: IpAddr,
    pub second_ip: IpAddr,
}

impl Default for DnsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            udp_listen: "0.0.0.0:5353".parse().expect("static socket address"),
            tcp_listen: "0.0.0.0:5353".parse().expect("static socket address"),
            domain: None,
            ttl: 1,
            first_ip: "192.0.2.1".parse().expect("static IP address"),
            second_ip: "127.0.0.1".parse().expect("static IP address"),
        }
    }
}

impl DnsConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if !self.enabled {
            return Ok(());
        }
        let domain = self
            .domain
            .as_deref()
            .ok_or_else(|| invalid("dns.domain is required when DNS is enabled"))?;
        if !valid_domain(domain) {
            return Err(invalid("dns.domain is not a valid DNS name"));
        }
        if self.ttl == 0 {
            return Err(invalid("dns.ttl must be greater than zero"));
        }
        if self.first_ip.is_ipv4() != self.second_ip.is_ipv4() {
            return Err(invalid(
                "dns.first_ip and dns.second_ip must use the same address family",
            ));
        }
        Ok(())
    }
}

fn env_value(name: &'static str) -> Option<String> {
    env::var_os(name).map(|value| value.to_string_lossy().into_owned())
}

fn parse_env<T>(name: &'static str, value: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error: T::Err| ConfigError::Environment {
            name,
            message: error.to_string(),
        })
}

fn nonempty_or_none(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn valid_domain(domain: &str) -> bool {
    let domain = domain.trim_end_matches('.');
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Validation(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_operational() {
        let config = Config::default();
        config.validate().unwrap();
        assert_eq!(config.server.listen.port(), 8080);
        assert_eq!(config.redirects.default_hops, 3);
        assert_eq!(config.redirects.max_hops, 10);
        assert!(config.catalog.external_packs.is_empty());
        assert!(!config.dns.enabled);
    }

    #[test]
    fn partial_yaml_gets_defaults() {
        let config = Config::from_yaml(
            r#"
server:
  listen: 127.0.0.1:9000
redirects:
  default_status: 307
  default_hops: 5
catalog:
  include_builtin: true
  required_tags: [cloud]
"#,
        )
        .unwrap();
        assert_eq!(config.server.listen.port(), 9000);
        assert_eq!(config.redirects.default_status, 307);
        assert_eq!(config.redirects.max_hops, 10);
        assert_eq!(config.catalog.required_tags, ["cloud"]);
    }

    #[test]
    fn unknown_yaml_keys_are_rejected() {
        let error = Config::from_yaml("server:\n  litsen: 127.0.0.1:9000\n").unwrap_err();
        assert!(error.to_string().contains("unknown field `litsen`"));
    }

    #[test]
    fn invalid_redirect_and_rebind_settings_are_rejected() {
        let error = Config::from_yaml("redirects:\n  default_hops: 11\n").unwrap_err();
        assert!(error.to_string().contains("default_hops"));

        let error = Config::from_yaml("dns:\n  enabled: true\n").unwrap_err();
        assert!(error.to_string().contains("domain is required"));
    }

    #[test]
    fn domain_validation_handles_edge_cases() {
        assert!(valid_domain("rebind.example.org"));
        assert!(valid_domain("rebind.example.org."));
        assert!(!valid_domain("-bad.example"));
        assert!(!valid_domain("bad..example"));
        assert!(!valid_domain("bad_.example"));
    }

    #[test]
    fn public_url_rejects_paths_queries_and_fragments() {
        for url in [
            "https://bend.example/tools/waybend",
            "https://bend.example/?tenant=one",
            "https://bend.example/#waybend",
        ] {
            let error = Config::from_yaml(&format!("server:\n  public_url: {url}\n")).unwrap_err();
            assert!(error.to_string().contains("must not contain"));
        }
    }

    #[test]
    fn cors_origins_are_strict_origins() {
        Config::from_yaml("server:\n  cors_origins: [https://console.example:8443]\n").unwrap();
        for yaml in [
            "server:\n  cors_origins: ['*', https://console.example]\n",
            "server:\n  cors_origins: [console.example]\n",
            "server:\n  cors_origins: [https://console.example/path]\n",
        ] {
            assert!(Config::from_yaml(yaml).is_err());
        }
    }
}
