use std::{collections::BTreeMap, future::Future, net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tower_http::{
    compression::CompressionLayer,
    cors::{AllowOrigin, Any, CorsLayer},
    trace::TraceLayer,
};
use uuid::Uuid;

use crate::{
    catalog::Catalog,
    config::Config,
    engine::{EngineError, build_redirect, redirect_status, render_template},
    model::{CatalogEntry, EvidenceEvent, RequestContext},
    storage::{EventQuery, Store},
};

const INDEX_HTML: &str = include_str!("../ui/index.html");
const APP_CSS: &str = include_str!("../ui/app.css");
const APP_JS: &str = include_str!("../ui/app.js");

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub catalog: Catalog,
    pub store: Store,
    admin_token: Option<Arc<str>>,
}

impl AppState {
    pub fn new(config: Config, catalog: Catalog, store: Store) -> Self {
        let admin_token = config
            .security
            .admin_token
            .clone()
            .or_else(|| std::env::var("WAYBEND_ADMIN_TOKEN").ok())
            .filter(|token| !token.is_empty())
            .map(Arc::<str>::from);
        Self {
            config,
            catalog,
            store,
            admin_token,
        }
    }

    pub fn with_admin_token(mut self, token: Option<String>) -> Self {
        self.admin_token = token
            .filter(|value| !value.is_empty())
            .map(Arc::<str>::from);
        self
    }
}

pub async fn run(config: Config, catalog: Catalog, store: Store) -> anyhow::Result<()> {
    run_until(config, catalog, store, shutdown_signal()).await
}

pub async fn run_until<F>(
    config: Config,
    catalog: Catalog,
    store: Store,
    shutdown: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listen = config.server.listen;
    let retention_days = config.storage.retention_days;
    let prune_store = store.clone();
    let pruned = storage_task(move || prune_store.prune(retention_days))
        .await
        .map_err(anyhow::Error::msg)?;
    if pruned > 0 {
        tracing::info!(pruned, retention_days, "pruned expired evidence");
    }
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let retention_store = store.clone();
    let retention_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 60 * 60));
        interval.tick().await;
        loop {
            interval.tick().await;
            let store = retention_store.clone();
            match storage_task(move || store.prune(retention_days)).await {
                Ok(removed) if removed > 0 => {
                    tracing::info!(removed, retention_days, "pruned expired evidence");
                }
                Err(error) => tracing::warn!(%error, "could not prune expired evidence"),
                _ => {}
            }
        }
    });
    tracing::info!(%listen, routes = catalog.entries().len(), "HTTP server listening");
    axum::serve(
        listener,
        router(AppState::new(config, catalog, store))
            .into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await?;
    retention_task.abort();
    Ok(())
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler must initialize");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub fn router(state: AppState) -> Router {
    let max_body_bytes = state
        .config
        .server
        .body_limit
        .min(state.config.storage.max_body_bytes);
    let cors_origins = state.config.server.cors_origins.clone();
    let mut app = Router::new()
        .route("/", get(index))
        .route("/assets/app.css", get(stylesheet))
        .route("/assets/app.js", get(javascript))
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/r/{id}", any(redirect_first))
        .route("/r/{id}/{hop}", any(redirect_hop))
        .route("/d/{status}/{hops}/{encoded}", any(dynamic_first))
        .route("/d/{status}/{hops}/{encoded}/{hop}", any(dynamic_hop))
        .route("/c/{token}", any(capture_callback))
        .route("/c/{token}/{*path}", any(capture_callback_path))
        .route("/api/v1/catalog", get(api_catalog))
        .route("/api/v1/routes", get(api_catalog))
        .route("/api/v1/encode", get(api_encode))
        .route("/api/v1/capabilities", get(api_capabilities))
        .route("/api/v1/events", get(api_events))
        .route("/api/v1/events/{id}", get(api_event))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state);
    if !cors_origins.is_empty() {
        let origin = if cors_origins.iter().any(|origin| origin == "*") {
            AllowOrigin::any()
        } else {
            let values = cors_origins
                .iter()
                .filter_map(|origin| origin.parse::<HeaderValue>().ok())
                .collect::<Vec<_>>();
            AllowOrigin::list(values)
        };
        app = app.layer(
            CorsLayer::new()
                .allow_origin(origin)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }
    app
}

async fn index() -> impl IntoResponse {
    static_asset("text/html; charset=utf-8", INDEX_HTML, "no-cache")
}

async fn stylesheet() -> impl IntoResponse {
    static_asset("text/css; charset=utf-8", APP_CSS, "public, max-age=3600")
}

async fn javascript() -> impl IntoResponse {
    static_asset(
        "text/javascript; charset=utf-8",
        APP_JS,
        "public, max-age=3600",
    )
}

fn static_asset(content_type: &'static str, body: &'static str, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CACHE_CONTROL, HeaderValue::from_static(cache)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
        ],
        body,
    )
        .into_response()
}

#[derive(Serialize)]
struct Health<'a> {
    status: &'a str,
    version: &'a str,
}

async fn liveness() -> impl IntoResponse {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

async fn readiness(State(state): State<AppState>) -> Response {
    let store = state.store.clone();
    match storage_task(move || store.health()).await {
        Ok(()) => Json(Health {
            status: "ready",
            version: env!("CARGO_PKG_VERSION"),
        })
        .into_response(),
        Err(error) => ApiError::unavailable(error.to_string()).into_response(),
    }
}

#[axum::debug_handler]
async fn redirect_first(
    State(state): State<AppState>,
    Path(id): Path<String>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    redirect_impl(state, id, 1, Some(connect), request).await
}

#[axum::debug_handler]
async fn redirect_hop(
    State(state): State<AppState>,
    Path((id, hop)): Path<(String, u8)>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    redirect_impl(state, id, hop, Some(connect), request).await
}

async fn redirect_impl(
    state: AppState,
    id: String,
    hop: u8,
    connect: Option<ConnectInfo<SocketAddr>>,
    request: Request,
) -> Response {
    let Some(route) = state.catalog.get(&id) else {
        return ApiError::not_found(
            "route_not_found",
            format!("redirect route {id:?} does not exist"),
        )
        .into_response();
    };
    let headers = request.headers();
    let token = request_token(request.uri()).unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let context = RequestContext {
        client_ip: client_ip(headers, connect, state.config.server.trust_proxy),
        referer_host: referer_host(headers),
        query: correlation_query(request.uri(), &token),
        token,
        public_url: public_url(&state.config, headers),
    };
    let mut redirect = match build_redirect(route, hop, &context) {
        Ok(response) => response,
        Err(EngineError::InvalidHop { max }) => {
            return ApiError::bad_request(
                "invalid_hop",
                format!("hop must be between 1 and {max}"),
            )
            .into_response();
        }
        Err(error) => {
            tracing::error!(route_id = %id, %error, "could not build redirect");
            return ApiError::internal(
                "invalid_route",
                "the configured route cannot produce a response",
            )
            .into_response();
        }
    };
    if redirect.final_hop
        && state.config.redirects.preserve_query
        && let Some(query) = request.uri().query()
    {
        redirect.location = append_query_before_fragment(&redirect.location, query);
    }

    let mut response = Body::empty().into_response();
    *response.status_mut() = redirect.status;
    let location = match HeaderValue::from_str(&redirect.location) {
        Ok(value) => value,
        Err(_) => {
            return ApiError::internal(
                "invalid_location",
                "the route produced an invalid Location header",
            )
            .into_response();
        }
    };
    response.headers_mut().insert(header::LOCATION, location);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    for (name, value) in redirect.headers {
        if !is_reserved_redirect_header(&name) {
            response.headers_mut().insert(name, value);
        }
    }

    let event = EvidenceEvent {
        id: Uuid::new_v4().to_string(),
        received_at: chrono::Utc::now(),
        kind: "redirect".into(),
        token: Some(context.token),
        route_id: Some(id),
        source_ip: context.client_ip,
        method: Some(request.method().to_string()),
        path: Some(request.uri().path().to_owned()),
        query: request.uri().query().map(str::to_owned),
        headers: headers_to_map(headers),
        body: None,
        dns_name: None,
        dns_type: None,
        dns_answer: None,
        dns_sequence: None,
    };
    let store = state.store.clone();
    if let Err(error) = storage_task(move || store.record(event)).await {
        tracing::error!(%error, "could not record redirect evidence");
    }
    response
}

#[axum::debug_handler]
async fn dynamic_first(
    State(state): State<AppState>,
    Path((status, hops, encoded)): Path<(u16, u8, String)>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    dynamic_impl(state, status, hops, encoded, 1, Some(connect), request).await
}

#[axum::debug_handler]
async fn dynamic_hop(
    State(state): State<AppState>,
    Path((status, hops, encoded, hop)): Path<(u16, u8, String, u8)>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    dynamic_impl(state, status, hops, encoded, hop, Some(connect), request).await
}

async fn dynamic_impl(
    state: AppState,
    status: u16,
    hops: u8,
    encoded: String,
    hop: u8,
    connect: Option<ConnectInfo<SocketAddr>>,
    request: Request,
) -> Response {
    if !state.config.redirects.allow_target_override {
        return ApiError::not_found("not_found", "endpoint does not exist").into_response();
    }
    if hops == 0 || hops > state.config.redirects.max_hops || hop == 0 || hop > hops {
        return ApiError::bad_request(
            "invalid_hop",
            format!(
                "hops and hop must be between 1 and {}",
                state.config.redirects.max_hops
            ),
        )
        .into_response();
    }
    let Some(status) = redirect_status(status) else {
        return ApiError::bad_request(
            "invalid_status",
            "status must be one of 301, 302, 303, 307, or 308",
        )
        .into_response();
    };
    let target = match URL_SAFE_NO_PAD.decode(encoded.as_bytes()) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(target) if !target.is_empty() => target,
            _ => {
                return ApiError::bad_request("invalid_target", "target is not valid UTF-8")
                    .into_response();
            }
        },
        Err(_) => {
            return ApiError::bad_request("invalid_target", "target is not valid base64url")
                .into_response();
        }
    };
    let headers = request.headers();
    let token = request_token(request.uri()).unwrap_or_else(|| Uuid::new_v4().simple().to_string());
    let context = RequestContext {
        client_ip: client_ip(headers, connect, state.config.server.trust_proxy),
        referer_host: referer_host(headers),
        query: correlation_query(request.uri(), &token),
        token,
        public_url: public_url(&state.config, headers),
    };
    let location = if hop == hops {
        match render_template(&target, &context) {
            Ok(target) => target,
            Err(error) => {
                return ApiError::bad_request("invalid_target", error.to_string()).into_response();
            }
        }
    } else {
        format!(
            "{}/d/{}/{}/{}/{}?{}",
            context.public_url.trim_end_matches('/'),
            status.as_u16(),
            hops,
            encoded,
            hop + 1,
            context.query
        )
    };
    let location = if hop == hops
        && state.config.redirects.preserve_query
        && let Some(query) = request.uri().query()
    {
        append_query_before_fragment(&location, query)
    } else {
        location
    };
    let Ok(location_header) = HeaderValue::from_str(&location) else {
        return ApiError::bad_request(
            "invalid_target",
            "target cannot be used in a Location header",
        )
        .into_response();
    };
    let mut response = Body::empty().into_response();
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::LOCATION, location_header);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    let event = EvidenceEvent {
        id: Uuid::new_v4().to_string(),
        received_at: chrono::Utc::now(),
        kind: "redirect".into(),
        token: Some(context.token),
        route_id: Some("dynamic".into()),
        source_ip: context.client_ip,
        method: Some(request.method().to_string()),
        path: Some(request.uri().path().to_owned()),
        query: request.uri().query().map(str::to_owned),
        headers: headers_to_map(headers),
        body: None,
        dns_name: None,
        dns_type: None,
        dns_answer: None,
        dns_sequence: None,
    };
    let store = state.store.clone();
    if let Err(error) = storage_task(move || store.record(event)).await {
        tracing::error!(%error, "could not record dynamic redirect evidence");
    }
    response
}

#[axum::debug_handler]
async fn capture_callback(
    State(state): State<AppState>,
    Path(token): Path<String>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    capture_callback_impl(state, token, connect, request).await
}

#[axum::debug_handler]
async fn capture_callback_path(
    State(state): State<AppState>,
    Path((token, _path)): Path<(String, String)>,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    capture_callback_impl(state, token, connect, request).await
}

async fn capture_callback_impl(
    state: AppState,
    token: String,
    connect: ConnectInfo<SocketAddr>,
    request: Request,
) -> Response {
    let Some(token) = valid_token(&token).then_some(token) else {
        return ApiError::bad_request(
            "invalid_token",
            "token must be 1–128 URL-safe ASCII characters",
        )
        .into_response();
    };
    let source_ip = client_ip(
        request.headers(),
        Some(connect),
        state.config.server.trust_proxy,
    );
    let (parts, body) = request.into_parts();
    let body_limit = state
        .config
        .server
        .body_limit
        .min(state.config.storage.max_body_bytes);
    let bytes = match to_bytes(body, body_limit).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return ApiError::payload_too_large(body_limit).into_response();
        }
    };
    let event_id = Uuid::new_v4().to_string();
    let event = EvidenceEvent {
        id: event_id.clone(),
        received_at: chrono::Utc::now(),
        kind: "http".into(),
        token: Some(token),
        route_id: None,
        source_ip,
        method: Some(parts.method.to_string()),
        path: Some(parts.uri.path().to_owned()),
        query: parts.uri.query().map(str::to_owned),
        headers: headers_to_map(&parts.headers),
        body: body_text(bytes),
        dns_name: None,
        dns_type: None,
        dns_answer: None,
        dns_sequence: None,
    };
    let store = state.store.clone();
    if let Err(error) = storage_task(move || store.record(event)).await {
        tracing::error!(%error, "could not persist callback evidence");
        return ApiError::internal("storage_error", "callback could not be persisted")
            .into_response();
    }
    (
        StatusCode::OK,
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"captured": true, "event_id": event_id})),
    )
        .into_response()
}

#[derive(Serialize)]
struct CatalogResponse<'a> {
    routes: &'a [CatalogEntry],
    count: usize,
}

#[derive(Debug, Deserialize)]
struct EncodeParams {
    target: String,
    status: Option<u16>,
    hops: Option<u8>,
}

#[derive(Debug, Serialize)]
struct EncodeResponse {
    url: String,
    encoded: String,
    status: u16,
    hops: u8,
}

async fn api_encode(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<EncodeParams>,
) -> Response {
    if let Err(error) = authorize(&state, &headers) {
        return error.into_response();
    }
    if !state.config.redirects.allow_target_override {
        return ApiError::not_found("not_found", "endpoint does not exist").into_response();
    }
    if params.target.is_empty() {
        return ApiError::bad_request("invalid_target", "target must not be empty").into_response();
    }
    let status = params
        .status
        .unwrap_or(state.config.redirects.default_status);
    if redirect_status(status).is_none() {
        return ApiError::bad_request(
            "invalid_status",
            "status must be one of 301, 302, 303, 307, or 308",
        )
        .into_response();
    }
    let hops = params.hops.unwrap_or(state.config.redirects.default_hops);
    if hops == 0 || hops > state.config.redirects.max_hops {
        return ApiError::bad_request(
            "invalid_hop",
            format!(
                "hops must be between 1 and {}",
                state.config.redirects.max_hops
            ),
        )
        .into_response();
    }
    let encoded = URL_SAFE_NO_PAD.encode(params.target.as_bytes());
    let base = public_url(&state.config, &headers);
    Json(EncodeResponse {
        url: format!("{}/d/{status}/{hops}/{encoded}", base.trim_end_matches('/')),
        encoded,
        status,
        hops,
    })
    .into_response()
}

#[derive(Serialize)]
struct Capabilities {
    version: &'static str,
    default_status: u16,
    default_hops: u8,
    max_hops: u8,
    dynamic_redirects: bool,
    dns_enabled: bool,
    dns_domain: Option<String>,
}

async fn api_capabilities(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state, &headers) {
        return error.into_response();
    }
    Json(Capabilities {
        version: env!("CARGO_PKG_VERSION"),
        default_status: state.config.redirects.default_status,
        default_hops: state.config.redirects.default_hops,
        max_hops: state.config.redirects.max_hops,
        dynamic_redirects: state.config.redirects.allow_target_override,
        dns_enabled: state.config.dns.enabled,
        dns_domain: state.config.dns.domain.clone(),
    })
    .into_response()
}

async fn api_catalog(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(error) = authorize(&state, &headers) {
        return error.into_response();
    }
    let routes = state.catalog.entries();
    Json(CatalogResponse {
        routes,
        count: routes.len(),
    })
    .into_response()
}

#[derive(Debug, Deserialize)]
struct EventsParams {
    limit: Option<usize>,
    offset: Option<usize>,
    token: Option<String>,
    kind: Option<String>,
    route_id: Option<String>,
    q: Option<String>,
}

#[derive(Serialize)]
struct EventsResponse {
    events: Vec<EvidenceEvent>,
    count: usize,
    total: u64,
}

async fn api_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<EventsParams>,
) -> Response {
    if let Err(error) = authorize(&state, &headers) {
        return error.into_response();
    }
    let query = EventQuery {
        kind: params.kind,
        token: params.token,
        route_id: params.route_id,
        search: params.q,
        limit: params.limit.unwrap_or(100),
        offset: params.offset.unwrap_or(0),
    };
    let list_store = state.store.clone();
    let list_query = query.clone();
    let count_store = state.store.clone();
    let count_query = query;
    let (events, total) = tokio::join!(
        storage_task(move || list_store.list(&list_query)),
        storage_task(move || count_store.count_filtered(&count_query))
    );
    match (events, total) {
        (Ok(events), Ok(total)) => {
            let count = events.len();
            Json(EventsResponse {
                events,
                count,
                total,
            })
            .into_response()
        }
        (Err(error), _) | (_, Err(error)) => {
            tracing::error!(%error, "could not query evidence");
            ApiError::internal("storage_error", "evidence could not be queried").into_response()
        }
    }
}

async fn api_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = authorize(&state, &headers) {
        return error.into_response();
    }
    let store = state.store.clone();
    match storage_task(move || store.get(&id)).await {
        Ok(Some(event)) => Json(event).into_response(),
        Ok(None) => {
            ApiError::not_found("event_not_found", "evidence event does not exist").into_response()
        }
        Err(error) => {
            tracing::error!(%error, "could not query evidence event");
            ApiError::internal("storage_error", "evidence could not be queried").into_response()
        }
    }
}

async fn storage_task<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> crate::storage::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| format!("storage worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = state.admin_token.as_deref() else {
        return Ok(());
    };
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let valid = supplied
        .filter(|value| value.len() == expected.len())
        .map(|value| value.as_bytes().ct_eq(expected.as_bytes()).into())
        .unwrap_or(false);
    if valid {
        Ok(())
    } else {
        Err(ApiError::unauthorized())
    }
}

fn client_ip(
    headers: &HeaderMap,
    connect: Option<ConnectInfo<SocketAddr>>,
    trust_proxy: bool,
) -> String {
    if trust_proxy && let Some(ip) = forwarded_ip(headers) {
        return ip;
    }
    connect
        .map(|ConnectInfo(address)| address.ip().to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn forwarded_ip(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
    {
        let candidate = value.trim();
        if candidate.parse::<std::net::IpAddr>().is_ok() {
            return Some(candidate.to_owned());
        }
    }
    let forwarded = forwarded_parameter(headers, "for")?;
    let candidate = forwarded.trim_matches(['[', ']']);
    candidate
        .parse::<std::net::IpAddr>()
        .ok()
        .map(|ip| ip.to_string())
}

fn referer_host(headers: &HeaderMap) -> String {
    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

fn public_url(config: &Config, headers: &HeaderMap) -> String {
    if let Some(url) = &config.server.public_url {
        return url.trim_end_matches('/').to_owned();
    }
    let direct_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<http::uri::Authority>().ok())
        .map(|authority| authority.to_string())
        .unwrap_or_else(|| config.server.listen.to_string());
    let forwarded_host = proxy_header_value(headers, "x-forwarded-host", "host");
    let host = if config.server.trust_proxy {
        forwarded_host
            .as_deref()
            .and_then(|value| value.parse::<http::uri::Authority>().ok())
            .map(|authority| authority.to_string())
            .unwrap_or(direct_host)
    } else {
        direct_host
    };
    let forwarded_proto = proxy_header_value(headers, "x-forwarded-proto", "proto");
    let scheme = if config.server.trust_proxy {
        forwarded_proto
            .as_deref()
            .filter(|value| matches!(*value, "http" | "https"))
            .unwrap_or("http")
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

fn forwarded_parameter(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("forwarded")?
        .to_str()
        .ok()?
        .split(',')
        .next()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().trim_matches('"').to_owned())
}

fn proxy_header_value(
    headers: &HeaderMap,
    forwarded_header: &str,
    forwarded_parameter_name: &str,
) -> Option<String> {
    headers
        .get(forwarded_header)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| forwarded_parameter(headers, forwarded_parameter_name))
}

fn request_token(uri: &Uri) -> Option<String> {
    url::form_urlencoded::parse(uri.query()?.as_bytes())
        .filter(|(key, _)| key == "token")
        .find_map(|(_, value)| valid_token(&value).then(|| value.into_owned()))
}

fn correlation_query(uri: &Uri, token: &str) -> String {
    let preserved = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|segment| {
            url::form_urlencoded::parse(segment.as_bytes())
                .next()
                .is_none_or(|(key, _)| key != "token")
        })
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("&");
    if preserved.is_empty() {
        format!("token={token}")
    } else {
        format!("{preserved}&token={token}")
    }
}

fn append_query_before_fragment(target: &str, query: &str) -> String {
    let (base, fragment) = target
        .split_once('#')
        .map_or((target, None), |(base, fragment)| (base, Some(fragment)));
    let mut output = String::with_capacity(target.len() + query.len() + 1);
    output.push_str(base);
    output.push(if base.contains('?') { '&' } else { '?' });
    output.push_str(query);
    if let Some(fragment) = fragment {
        output.push('#');
        output.push_str(fragment);
    }
    output
}

fn is_reserved_redirect_header(name: &http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "location"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "upgrade"
            | "cache-control"
    )
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

fn headers_to_map(headers: &HeaderMap) -> BTreeMap<String, Vec<String>> {
    let mut output = BTreeMap::<String, Vec<String>>::new();
    for (name, value) in headers {
        output.entry(name.to_string()).or_default().push(
            value
                .to_str()
                .map(str::to_owned)
                .unwrap_or_else(|_| format!("base64:{}", BASE64.encode(value.as_bytes()))),
        );
    }
    output
}

fn body_text(bytes: Bytes) -> Option<String> {
    if bytes.is_empty() {
        None
    } else {
        Some(match String::from_utf8(bytes.to_vec()) {
            Ok(text) => text,
            Err(_) => format!("base64:{}", BASE64.encode(bytes)),
        })
    }
}

async fn not_found() -> Response {
    ApiError::not_found("not_found", "endpoint does not exist").into_response()
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
        }
    }
    fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "unauthorized",
            message: "a valid bearer token is required".into(),
        }
    }
    fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code,
            message: message.into(),
        }
    }
    fn payload_too_large(limit: usize) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "body_too_large",
            message: format!("request body exceeds the {limit} byte capture limit"),
        }
    }
    fn internal(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code,
            message: message.into(),
        }
    }
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "not_ready",
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code: self.code,
                    message: self.message,
                },
            }),
        )
            .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Bearer realm=\"waybend\""),
            );
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use http::Method;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    fn app(token: Option<&str>) -> (Router, Store, String) {
        let config = Config::default();
        let catalog = Catalog::builtin(&config).unwrap();
        let route_id = catalog.entries()[0].id.clone();
        let store = Store::memory().unwrap();
        let state = AppState::new(config, catalog, store.clone())
            .with_admin_token(token.map(str::to_owned));
        (router(state), store, route_id)
    }

    fn request(method: Method, uri: impl AsRef<str>, body: Body) -> Request {
        Request::builder()
            .method(method)
            .uri(uri.as_ref())
            .extension(ConnectInfo("192.0.2.4:4100".parse::<SocketAddr>().unwrap()))
            .body(body)
            .unwrap()
    }

    #[test]
    fn accepts_only_safe_tokens() {
        assert!(valid_token("case-1_ok.example~x"));
        assert!(!valid_token("line\nbreak"));
        assert!(!valid_token(""));

        let uri: Uri = "/r/test?token=bad%0Avalue&x=%2Fraw&token=case-2"
            .parse()
            .unwrap();
        assert_eq!(request_token(&uri).as_deref(), Some("case-2"));
        assert_eq!(correlation_query(&uri, "case-2"), "x=%2Fraw&token=case-2");
    }

    #[test]
    fn trusts_forwarded_address_only_when_configured() {
        let headers = HeaderMap::from_iter([(
            "x-forwarded-for".parse().unwrap(),
            "203.0.113.5, 10.0.0.2".parse().unwrap(),
        )]);
        let peer = Some(ConnectInfo("192.0.2.4:4000".parse().unwrap()));
        assert_eq!(client_ip(&headers, peer, true), "203.0.113.5");
        assert_eq!(client_ip(&headers, peer, false), "192.0.2.4");

        let proxy_headers = HeaderMap::from_iter([
            (header::HOST, "waybend:8080".parse().unwrap()),
            (
                "x-forwarded-host".parse().unwrap(),
                "bend.example".parse().unwrap(),
            ),
            (
                "x-forwarded-proto".parse().unwrap(),
                "https".parse().unwrap(),
            ),
        ]);
        let mut config = Config::default();
        config.server.trust_proxy = true;
        assert_eq!(public_url(&config, &proxy_headers), "https://bend.example");
    }

    #[tokio::test]
    async fn redirects_arbitrary_methods_and_records_evidence() {
        let (app, store, route_id) = app(None);
        let response = app
            .oneshot(request(
                Method::PATCH,
                format!("/r/{route_id}/1?token=case-7"),
                Body::empty(),
            ))
            .await
            .unwrap();
        assert!(response.status().is_redirection());
        assert!(response.headers().contains_key(header::LOCATION));
        let events = store.list_recent(10, Some("case-7")).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].method.as_deref(), Some("PATCH"));
    }

    #[tokio::test]
    async fn builds_dynamic_multihop_redirects_without_normalizing_target() {
        let (app, store, _) = app(None);
        let target = "Gopher://127.0.0.1:6379/_INFO%0D%0APING";
        let encoded = URL_SAFE_NO_PAD.encode(target.as_bytes());
        let first = app
            .clone()
            .oneshot(request(
                Method::GET,
                format!("/d/307/2/{encoded}?token=dynamic-1"),
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            first.headers()[header::LOCATION],
            format!("http://0.0.0.0:8080/d/307/2/{encoded}/2?token=dynamic-1")
        );

        let final_response = app
            .oneshot(request(
                Method::POST,
                format!("/d/307/2/{encoded}/2?token=dynamic-1"),
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_eq!(final_response.headers()[header::LOCATION], target);
        assert_eq!(store.list_recent(10, Some("dynamic-1")).unwrap().len(), 2);
    }

    #[test]
    fn preserved_query_is_inserted_before_fragments() {
        assert_eq!(
            append_query_before_fragment("http://127.0.0.1/path#anchor", "token=case-1"),
            "http://127.0.0.1/path?token=case-1#anchor"
        );
        assert_eq!(
            append_query_before_fragment("http://127.0.0.1/?x=1#anchor", "token=case-1"),
            "http://127.0.0.1/?x=1&token=case-1#anchor"
        );
    }

    #[tokio::test]
    async fn captures_callback_body_and_protects_admin_api() {
        let (app, store, _) = app(Some("secret"));
        let response = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/c/case-8/nested/path?x=1",
                Body::from("proof"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            store.list_recent(10, Some("case-8")).unwrap()[0]
                .body
                .as_deref(),
            Some("proof")
        );

        let denied = app
            .clone()
            .oneshot(request(Method::GET, "/api/v1/events", Body::empty()))
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

        let allowed = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/events")
                    .header(header::AUTHORIZATION, "Bearer secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        let payload = allowed.into_body().collect().await.unwrap().to_bytes();
        assert!(serde_json::from_slice::<serde_json::Value>(&payload).is_ok());
    }
}
