use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RouteDefinition {
    pub id: String,
    pub title: String,
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub target: String,
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default = "default_hops")]
    pub hops: u8,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_status() -> u16 {
    302
}
fn default_hops() -> u8 {
    3
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogEntry {
    pub id: String,
    pub title: String,
    pub category: String,
    pub description: String,
    pub tags: Vec<String>,
    pub target: String,
    pub status: u16,
    pub hops: u8,
    pub headers: BTreeMap<String, String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEvent {
    pub id: String,
    pub received_at: DateTime<Utc>,
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

#[derive(Debug, Clone)]
pub struct RequestContext {
    pub client_ip: String,
    pub referer_host: String,
    pub token: String,
    pub public_url: String,
    /// Stable query string carried across redirect hops for correlation.
    pub query: String,
}
