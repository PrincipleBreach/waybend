use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use clap::{Args, Parser, Subcommand, ValueEnum};
use tokio::sync::{mpsc, watch};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use crate::{
    catalog::{Catalog, CatalogFilter},
    config::Config,
    dns::{DnsConfig, DnsServer, EvidenceHook},
    model::EvidenceEvent,
    server,
    storage::{EventQuery, Store},
};

#[derive(Debug, Parser)]
#[command(
    name = "waybend",
    version,
    about = "Self-hosted SSRF request-bending workbench"
)]
pub struct Cli {
    #[arg(long, global = true, env = "WAYBEND_LOG", default_value = "info")]
    log: String,
    #[arg(
        long,
        global = true,
        env = "WAYBEND_LOG_FORMAT",
        value_enum,
        default_value = "text"
    )]
    log_format: LogFormat,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogFormat {
    Text,
    Json,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP workbench and optional authoritative DNS server.
    Serve(ConfigPath),
    /// Print, filter, or export the compiled payload catalog.
    Catalog(CatalogArgs),
    /// Build a copy-ready dynamic redirect URL for any target.
    Encode(EncodeArgs),
    /// Validate configuration, packs, routes, and DNS settings.
    Validate(ConfigPath),
    /// Write a documented starter configuration.
    Init(InitArgs),
    /// Query or prune captured HTTP and DNS evidence.
    Evidence(EvidenceArgs),
    /// Run the Model Context Protocol server over standard input/output.
    Mcp(ConfigPath),
    /// Print build version information.
    Version,
}

#[derive(Debug, Args)]
struct ConfigPath {
    #[arg(short, long, env = "WAYBEND_CONFIG")]
    config: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct CatalogArgs {
    #[arg(short, long, env = "WAYBEND_CONFIG")]
    config: Option<PathBuf>,
    #[arg(short, long)]
    query: Option<String>,
    #[arg(short = 'C', long)]
    category: Option<String>,
    #[arg(long, value_delimiter = ',')]
    tag: Vec<String>,
    #[arg(short, long, value_enum, default_value = "text")]
    format: CatalogFormat,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CatalogFormat {
    Text,
    Json,
    Urls,
    Csv,
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(default_value = "waybend.yml")]
    output: PathBuf,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct EncodeArgs {
    /// Exact terminal target, including non-HTTP schemes and encoded bytes.
    target: String,
    #[arg(short, long, env = "WAYBEND_CONFIG")]
    config: Option<PathBuf>,
    #[arg(long)]
    status: Option<u16>,
    #[arg(long)]
    hops: Option<u8>,
}

#[derive(Debug, Args)]
struct EvidenceArgs {
    #[arg(short, long, env = "WAYBEND_CONFIG")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: EvidenceCommand,
}

#[derive(Debug, Subcommand)]
enum EvidenceCommand {
    /// List captured events, newest first.
    List {
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        token: Option<String>,
        #[arg(long)]
        route: Option<String>,
        #[arg(short, long)]
        search: Option<String>,
        #[arg(short, long, default_value_t = 50)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long)]
        json: bool,
    },
    /// Remove events older than the configured retention period.
    Prune,
}

pub async fn run() -> Result<()> {
    run_with(Cli::parse()).await
}

async fn run_with(cli: Cli) -> Result<()> {
    init_tracing(&cli.log, cli.log_format)?;
    match cli.command {
        Command::Serve(args) => serve(args.config.as_deref()).await,
        Command::Catalog(args) => print_catalog(args),
        Command::Encode(args) => encode(args),
        Command::Validate(args) => validate(args.config.as_deref()),
        Command::Init(args) => init_config(&args.output, args.force),
        Command::Evidence(args) => evidence(args),
        Command::Mcp(args) => serve_mcp(args.config.as_deref()).await,
        Command::Version => {
            println!("waybend {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn serve_mcp(path: Option<&Path>) -> Result<()> {
    let (config, catalog) = load(path)?;
    if let Some(parent) = config
        .storage
        .database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let store = Store::open(&config.storage.database)
        .with_context(|| format!("could not open {}", config.storage.database.display()))?;
    crate::mcp::serve(config, catalog, store).await
}

fn init_tracing(filter: &str, format: LogFormat) -> Result<()> {
    let filter = EnvFilter::try_new(filter).context("invalid log filter")?;
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    match format {
        LogFormat::Text => builder
            .try_init()
            .map_err(|error| anyhow::anyhow!("could not initialize logging: {error}"))?,
        LogFormat::Json => builder
            .json()
            .try_init()
            .map_err(|error| anyhow::anyhow!("could not initialize logging: {error}"))?,
    }
    Ok(())
}

fn load(path: Option<&Path>) -> Result<(Config, Catalog)> {
    let config = Config::load(path)?;
    let catalog = Catalog::load(&config)?;
    Ok((config, catalog))
}

async fn serve(path: Option<&Path>) -> Result<()> {
    let (config, catalog) = load(path)?;
    if let Some(parent) = config
        .storage
        .database
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("could not create data directory {}", parent.display()))?;
    }
    let store = Store::open(&config.storage.database)
        .with_context(|| format!("could not open {}", config.storage.database.display()))?;

    if !config.dns.enabled {
        return server::run(config, catalog, store).await;
    }

    let dns_config = DnsConfig::try_from(&config.dns)?;
    let (dns_event_tx, mut dns_event_rx) = mpsc::channel(4_096);
    let hook: EvidenceHook = Arc::new(move |event| {
        if dns_event_tx.try_send(event).is_err() {
            tracing::warn!("DNS evidence queue is full; dropping event");
        }
    });
    let dns_server = DnsServer::new(dns_config, Some(hook));
    let dns_store = store.clone();
    let dns_writer = tokio::spawn(async move {
        while let Some(event) = dns_event_rx.recv().await {
            let evidence = EvidenceEvent {
                id: Uuid::new_v4().to_string(),
                received_at: Utc::now(),
                kind: "dns".into(),
                token: event.token,
                route_id: None,
                source_ip: event.source.ip().to_string(),
                method: None,
                path: None,
                query: None,
                headers: BTreeMap::new(),
                body: None,
                dns_name: Some(event.name),
                dns_type: Some(event.record_type.to_string()),
                dns_answer: event.answer.map(|answer| answer.to_string()),
                dns_sequence: Some(event.sequence),
            };
            let store = dns_store.clone();
            match tokio::task::spawn_blocking(move || store.record(evidence)).await {
                Ok(Err(error)) => tracing::warn!(%error, "could not persist DNS evidence"),
                Err(error) => tracing::warn!(%error, "DNS evidence worker failed"),
                _ => {}
            }
        }
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        server::shutdown_signal().await;
        let _ = shutdown_tx.send(true);
    });
    let mut http_shutdown = shutdown_rx.clone();
    tokio::try_join!(
        async {
            server::run_until(config, catalog, store, async move {
                let _ = http_shutdown.changed().await;
            })
            .await
        },
        async {
            dns_server
                .run_until(shutdown_rx)
                .await
                .map_err(anyhow::Error::from)
        }
    )?;
    dns_writer.await?;
    Ok(())
}

fn validate(path: Option<&Path>) -> Result<()> {
    let (config, catalog) = load(path)?;
    println!(
        "valid · {} routes · HTTP {} · DNS {}",
        catalog.entries().len(),
        config.server.listen,
        if config.dns.enabled {
            "enabled"
        } else {
            "disabled"
        }
    );
    Ok(())
}

fn print_catalog(args: CatalogArgs) -> Result<()> {
    let (config, catalog) = load(args.config.as_deref())?;
    let entries = catalog.filter(&CatalogFilter {
        query: args.query,
        category: args.category,
        tags: args.tag,
    });
    match args.format {
        CatalogFormat::Json => println!("{}", serde_json::to_string_pretty(&entries)?),
        CatalogFormat::Urls => {
            let base = config
                .server
                .public_url
                .as_deref()
                .unwrap_or("http://localhost:8080")
                .trim_end_matches('/');
            for entry in entries {
                println!("{base}/r/{}", entry.id);
            }
        }
        CatalogFormat::Csv => {
            println!("id,category,title,status,hops,target,tags");
            for entry in entries {
                println!(
                    "{},{},{},{},{},{},{}",
                    csv(&entry.id),
                    csv(&entry.category),
                    csv(&entry.title),
                    entry.status,
                    entry.hops,
                    csv(&entry.target),
                    csv(&entry.tags.join(";"))
                );
            }
        }
        CatalogFormat::Text => {
            for entry in &entries {
                println!("{:<48} {:<18} {}", entry.id, entry.category, entry.target);
            }
            eprintln!("{} routes", entries.len());
        }
    }
    Ok(())
}

fn encode(args: EncodeArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    if !config.redirects.allow_target_override {
        bail!("dynamic target overrides are disabled by configuration");
    }
    let status = args.status.unwrap_or(config.redirects.default_status);
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        bail!("status must be one of 301, 302, 303, 307, or 308");
    }
    let hops = args.hops.unwrap_or(config.redirects.default_hops);
    if hops == 0 || hops > config.redirects.max_hops {
        bail!("hops must be between 1 and {}", config.redirects.max_hops);
    }
    let base = config
        .server
        .public_url
        .as_deref()
        .unwrap_or("http://localhost:8080")
        .trim_end_matches('/');
    let encoded = URL_SAFE_NO_PAD.encode(args.target.as_bytes());
    println!("{base}/d/{status}/{hops}/{encoded}");
    Ok(())
}

fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn init_config(output: &Path, force: bool) -> Result<()> {
    if output.exists() && !force {
        bail!(
            "{} already exists; use --force to replace it",
            output.display()
        );
    }
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, include_str!("../waybend.example.yml"))
        .with_context(|| format!("could not write {}", output.display()))?;
    println!("wrote {}", output.display());
    Ok(())
}

fn evidence(args: EvidenceArgs) -> Result<()> {
    let config = Config::load(args.config.as_deref())?;
    let store = Store::open(&config.storage.database)
        .with_context(|| format!("could not open {}", config.storage.database.display()))?;
    match args.command {
        EvidenceCommand::List {
            kind,
            token,
            route,
            search,
            limit,
            offset,
            json,
        } => {
            let events = store.list(&EventQuery {
                kind,
                token,
                route_id: route,
                search,
                limit,
                offset,
            })?;
            if json {
                println!("{}", serde_json::to_string_pretty(&events)?);
            } else {
                for event in events {
                    println!(
                        "{}\t{}\t{}\t{}",
                        event.received_at.to_rfc3339(),
                        event.kind,
                        event.token.as_deref().unwrap_or("-"),
                        event
                            .path
                            .as_deref()
                            .or(event.dns_name.as_deref())
                            .unwrap_or("-")
                    );
                }
            }
        }
        EvidenceCommand::Prune => {
            let removed = store.prune(config.storage.retention_days)?;
            println!("pruned {removed} events");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_fields() {
        assert_eq!(csv("a,\"b\""), "\"a,\"\"b\"\"\"");
    }
}
