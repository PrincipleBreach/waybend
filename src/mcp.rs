use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    catalog::{Catalog, CatalogFilter},
    config::Config,
    storage::{EventQuery, Store},
};

#[derive(Debug, Clone)]
pub struct WaybendMcp {
    config: Config,
    catalog: Catalog,
    store: Store,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchPayloads {
    /// Free-text match against id, title, description, target, and tags.
    pub query: Option<String>,
    /// Exact catalog category.
    pub category: Option<String>,
    /// Entries must contain every supplied tag.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Maximum results, from 1 to 100.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPayload {
    /// Exact catalog route id.
    pub id: String,
    /// Optional URL-safe correlation token.
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuildRedirect {
    /// Exact terminal URI. Encoded bytes and scheme casing are preserved.
    pub target: String,
    /// One of 301, 302, 303, 307, or 308.
    pub status: Option<u16>,
    /// Redirect chain length within the configured maximum.
    pub hops: Option<u8>,
    /// Optional URL-safe correlation token.
    pub token: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEvidence {
    pub token: Option<String>,
    pub kind: Option<String>,
    pub route_id: Option<String>,
    pub search: Option<String>,
    /// Maximum results, from 1 to 200.
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DnsRebindName {
    /// `static`, `alt`, or `toctou`.
    pub mode: String,
    /// Optional URL-safe correlation token.
    pub token: Option<String>,
    /// Optional first answer, using the configured address when omitted.
    pub first_ip: Option<String>,
    /// Optional second answer, using the configured address when omitted.
    pub second_ip: Option<String>,
    /// A-query count before switching in `toctou` mode. Defaults to 2.
    pub switch_after: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PayloadResult {
    pub id: String,
    pub title: String,
    pub category: String,
    pub description: String,
    pub tags: Vec<String>,
    pub target: String,
    pub url: String,
    pub status: u16,
    pub hops: u8,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RedirectResult {
    pub url: String,
    pub target: String,
    pub status: u16,
    pub hops: u8,
    pub token: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EvidenceResult {
    pub id: String,
    pub received_at: String,
    pub kind: String,
    pub token: Option<String>,
    pub route_id: Option<String>,
    pub source_ip: String,
    pub method: Option<String>,
    pub path: Option<String>,
    pub query: Option<String>,
    pub headers: BTreeMap<String, Vec<String>>,
    pub body: Option<String>,
    pub dns_name: Option<String>,
    pub dns_type: Option<String>,
    pub dns_answer: Option<String>,
    pub dns_sequence: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DnsNameResult {
    pub hostname: String,
    pub mode: String,
    pub first_ip: String,
    pub second_ip: String,
    pub switch_after: Option<u64>,
}

impl WaybendMcp {
    pub fn new(config: Config, catalog: Catalog, store: Store) -> Self {
        Self {
            config,
            catalog,
            store,
            tool_router: Self::tool_router(),
        }
    }

    fn base_url(&self) -> &str {
        self.config
            .server
            .public_url
            .as_deref()
            .unwrap_or("http://localhost:8080")
            .trim_end_matches('/')
    }

    fn payload(&self, entry: &crate::model::CatalogEntry, token: Option<&str>) -> PayloadResult {
        let suffix = token.map_or_else(String::new, |token| format!("?token={token}"));
        PayloadResult {
            id: entry.id.clone(),
            title: entry.title.clone(),
            category: entry.category.clone(),
            description: entry.description.clone(),
            tags: entry.tags.clone(),
            target: entry.target.clone(),
            url: format!("{}/r/{}{}", self.base_url(), entry.id, suffix),
            status: entry.status,
            hops: entry.hops,
            headers: entry.headers.clone(),
        }
    }
}

#[tool_router(router = tool_router)]
impl WaybendMcp {
    #[tool(
        name = "search_payloads",
        description = "Search Waybend's deterministic SSRF payload catalog by technique, category, or tags."
    )]
    pub async fn search_payloads(
        &self,
        Parameters(params): Parameters<SearchPayloads>,
    ) -> Result<Json<Vec<PayloadResult>>, String> {
        let limit = params.limit.unwrap_or(25).clamp(1, 100);
        let entries = self.catalog.filter(&CatalogFilter {
            query: params.query,
            category: params.category,
            tags: params.tags,
        });
        Ok(Json(
            entries
                .into_iter()
                .take(limit)
                .map(|entry| self.payload(entry, None))
                .collect(),
        ))
    }

    #[tool(
        name = "get_payload",
        description = "Resolve one catalog id into a copy-ready URL and its exact terminal target."
    )]
    pub async fn get_payload(
        &self,
        Parameters(params): Parameters<GetPayload>,
    ) -> Result<Json<PayloadResult>, String> {
        validate_token(params.token.as_deref())?;
        let entry = self
            .catalog
            .get(&params.id)
            .ok_or_else(|| format!("unknown payload id: {}", params.id))?;
        Ok(Json(self.payload(entry, params.token.as_deref())))
    }

    #[tool(
        name = "build_redirect",
        description = "Build a byte-preserving multi-hop redirect URL for any URI scheme."
    )]
    pub async fn build_redirect(
        &self,
        Parameters(params): Parameters<BuildRedirect>,
    ) -> Result<Json<RedirectResult>, String> {
        if !self.config.redirects.allow_target_override {
            return Err("dynamic redirects are disabled".into());
        }
        validate_token(params.token.as_deref())?;
        if params.target.is_empty() {
            return Err("target must not be empty".into());
        }
        let status = params
            .status
            .unwrap_or(self.config.redirects.default_status);
        if crate::engine::redirect_status(status).is_none() {
            return Err("status must be one of 301, 302, 303, 307, or 308".into());
        }
        let hops = params.hops.unwrap_or(self.config.redirects.default_hops);
        if hops == 0 || hops > self.config.redirects.max_hops {
            return Err(format!(
                "hops must be between 1 and {}",
                self.config.redirects.max_hops
            ));
        }
        let encoded = URL_SAFE_NO_PAD.encode(params.target.as_bytes());
        let suffix = params
            .token
            .as_deref()
            .map_or_else(String::new, |token| format!("?token={token}"));
        Ok(Json(RedirectResult {
            url: format!("{}/d/{status}/{hops}/{encoded}{suffix}", self.base_url()),
            target: params.target,
            status,
            hops,
            token: params.token,
        }))
    }

    #[tool(
        name = "list_evidence",
        description = "Read correlated HTTP redirect, callback, and DNS observations from Waybend."
    )]
    pub async fn list_evidence(
        &self,
        Parameters(params): Parameters<ListEvidence>,
    ) -> Result<Json<Vec<EvidenceResult>>, String> {
        validate_token(params.token.as_deref())?;
        let store = self.store.clone();
        let query = EventQuery {
            token: params.token,
            kind: params.kind,
            route_id: params.route_id,
            search: params.search,
            limit: params.limit.unwrap_or(50).clamp(1, 200),
            offset: 0,
        };
        let events = tokio::task::spawn_blocking(move || store.list(&query))
            .await
            .map_err(|error| format!("storage worker failed: {error}"))?
            .map_err(|error| error.to_string())?;
        Ok(Json(
            events
                .into_iter()
                .map(|event| EvidenceResult {
                    id: event.id,
                    received_at: event.received_at.to_rfc3339(),
                    kind: event.kind,
                    token: event.token,
                    route_id: event.route_id,
                    source_ip: event.source_ip,
                    method: event.method,
                    path: event.path,
                    query: event.query,
                    headers: event.headers,
                    body: event.body,
                    dns_name: event.dns_name,
                    dns_type: event.dns_type,
                    dns_answer: event.dns_answer,
                    dns_sequence: event.dns_sequence,
                })
                .collect(),
        ))
    }

    #[tool(
        name = "dns_rebind_name",
        description = "Build a Waybend static, alternating, or time-of-check/time-of-use DNS hostname."
    )]
    pub async fn dns_rebind_name(
        &self,
        Parameters(params): Parameters<DnsRebindName>,
    ) -> Result<Json<DnsNameResult>, String> {
        validate_dns_token(params.token.as_deref())?;
        let zone = self
            .config
            .dns
            .enabled
            .then_some(self.config.dns.domain.as_deref())
            .flatten()
            .ok_or("authoritative DNS is not configured")?
            .trim_end_matches('.');
        let first = params
            .first_ip
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "first_ip is not an IP address")?
            .unwrap_or(self.config.dns.first_ip);
        let second = params
            .second_ip
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "second_ip is not an IP address")?
            .unwrap_or(self.config.dns.second_ip);
        if first.is_ipv4() != second.is_ipv4() {
            return Err("first_ip and second_ip must use the same address family".into());
        }
        let token = params
            .token
            .as_deref()
            .map_or_else(String::new, |token| format!("t-{token}."));
        let first_label = encode_ip(first);
        let second_label = encode_ip(second);
        let (pattern, switch_after) = match params.mode.as_str() {
            "static" => (format!("{first_label}.static"), None),
            "alt" => (format!("{first_label}.{second_label}.alt"), None),
            "toctou" => {
                let count = params.switch_after.unwrap_or(2).max(1);
                (
                    format!("{first_label}.{second_label}.{count}.toctou"),
                    Some(count),
                )
            }
            _ => return Err("mode must be static, alt, or toctou".into()),
        };
        Ok(Json(DnsNameResult {
            hostname: format!("{token}{pattern}.{zone}"),
            mode: params.mode,
            first_ip: first.to_string(),
            second_ip: second.to_string(),
            switch_after,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for WaybendMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("waybend", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Search payloads, generate a unique correlation token, execute the returned URL against the scoped target, then query evidence with the same token. Preserve target bytes exactly."
            )
    }
}

pub async fn serve(config: Config, catalog: Catalog, store: Store) -> anyhow::Result<()> {
    let service = WaybendMcp::new(config, catalog, store)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

fn validate_token(token: Option<&str>) -> Result<(), String> {
    if token.is_some_and(|token| {
        token.is_empty()
            || token.len() > 128
            || !token.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
    }) {
        Err("token must be 1-128 URL-safe ASCII characters".into())
    } else {
        Ok(())
    }
}

fn validate_dns_token(token: Option<&str>) -> Result<(), String> {
    if token.is_some_and(|token| {
        token.is_empty()
            || token.len() > 61
            || !token
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || token.starts_with('-')
            || token.ends_with('-')
    }) {
        Err("DNS token must be a lowercase 1-61 character DNS label using letters, digits, or internal hyphens".into())
    } else {
        Ok(())
    }
}

fn encode_ip(ip: std::net::IpAddr) -> String {
    match ip {
        std::net::IpAddr::V4(ip) => ip.to_string().replace('.', "-"),
        std::net::IpAddr::V6(ip) => format!("{:032x}", u128::from(ip)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> WaybendMcp {
        let mut config = Config::default();
        config.server.public_url = Some("https://bend.example".into());
        config.dns.enabled = true;
        config.dns.domain = Some("rb.example".into());
        let catalog = Catalog::builtin(&config).unwrap();
        WaybendMcp::new(config, catalog, Store::memory().unwrap())
    }

    #[tokio::test]
    async fn exposes_five_deterministic_tools() {
        let tools = service().tool_router.list_all();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "build_redirect",
                "dns_rebind_name",
                "get_payload",
                "list_evidence",
                "search_payloads"
            ]
        );
    }

    #[tokio::test]
    async fn builds_exact_redirect_and_rebind_names() {
        let service = service();
        let redirect = service
            .build_redirect(Parameters(BuildRedirect {
                target: "gopher://127.0.0.1:6379/_PING%0D%0A".into(),
                status: Some(307),
                hops: Some(2),
                token: Some("case-1".into()),
            }))
            .await
            .unwrap();
        assert_eq!(redirect.0.target, "gopher://127.0.0.1:6379/_PING%0D%0A");
        assert!(redirect.0.url.ends_with("?token=case-1"));

        let dns = service
            .dns_rebind_name(Parameters(DnsRebindName {
                mode: "toctou".into(),
                token: Some("case-1".into()),
                first_ip: None,
                second_ip: None,
                switch_after: Some(3),
            }))
            .await
            .unwrap();
        assert_eq!(
            dns.0.hostname,
            "t-case-1.192-0-2-1.127-0-0-1.3.toctou.rb.example"
        );
        assert!(
            service
                .dns_rebind_name(Parameters(DnsRebindName {
                    mode: "alt".into(),
                    token: Some("Not_DNS_Safe".into()),
                    first_ip: None,
                    second_ip: None,
                    switch_after: None,
                }))
                .await
                .is_err()
        );
    }
}
