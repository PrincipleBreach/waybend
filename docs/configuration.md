# Configuration reference

Waybend loads YAML first, applies supported `WAYBEND_*` environment variables, then validates the complete result. Unknown YAML fields are errors.

## `server`

| Field | Default | Meaning |
|---|---:|---|
| `listen` | `0.0.0.0:8080` | HTTP socket. |
| `public_url` | unset | External base URL used in generated hops and templates. |
| `trust_proxy` | `false` | Use forwarded client/protocol/host values supplied by a trusted proxy. |
| `body_limit` | `1048576` | Hard HTTP request-body bound. |
| `cors_origins` | `[]` | Origins permitted to make browser cross-origin requests. |

`public_url` must be an absolute HTTP or HTTPS origin without a path, query, or
fragment. Set it in deployments with a reverse proxy. Enable `trust_proxy` only
when direct access to Waybend is restricted to the proxy. CORS entries use the
same origin form; `*` must be the only entry when used.

## `redirects`

| Field | Default | Meaning |
|---|---:|---|
| `default_status` | `302` | Default redirect status: 301, 302, 303, 307, or 308. |
| `default_hops` | `3` | Default chain length. |
| `max_hops` | `10` | Maximum accepted chain length. |
| `preserve_query` | `false` | Append inbound query data to a catalog route's final target. |
| `allow_target_override` | `true` | Enable path-encoded dynamic redirects and the encoder API. |

Route definitions can override status and hop count within validated limits.

## `catalog`

| Field | Default | Meaning |
|---|---:|---|
| `include_builtin` | `true` | Load routes compiled into Waybend. |
| `external_packs` | `[]` | Ordered YAML pack paths. |
| `enabled_categories` | `[]` | Category allowlist; empty means all. |
| `required_tags` | `[]` | Keep only routes containing required tags. |
| `disabled_ids` | `[]` | Route IDs excluded after packs load. |

Route IDs must be unique after all sources load. Filters apply to built-in and external routes.

## `storage`

| Field | Default | Meaning |
|---|---:|---|
| `database` | `./data/waybend.sqlite3` | SQLite evidence database path. |
| `retention_days` | `30` | Evidence retention window. |
| `max_body_bytes` | `262144` | Maximum callback body retained per event. |

## `dns`

| Field | Default | Meaning |
|---|---:|---|
| `enabled` | `false` | Start authoritative UDP and TCP DNS. |
| `udp_listen` | `0.0.0.0:5353` | UDP DNS socket. Production delegation normally maps port 53 to this socket. |
| `tcp_listen` | `0.0.0.0:5353` | TCP DNS socket. |
| `domain` | unset | Delegated DNS zone, such as `rb.example.com`. |
| `ttl` | `1` | Answer TTL in seconds. |
| `first_ip` | `192.0.2.1` | First address for configured rebinding behavior. |
| `second_ip` | `127.0.0.1` | Second address for configured rebinding behavior. |

## `security`

| Field | Default | Meaning |
|---|---:|---|
| `admin_token` | unset | Bearer token required by protected administrative endpoints. |

## Environment overrides

| Variable | Configuration field |
|---|---|
| `WAYBEND_LISTEN` | `server.listen` |
| `WAYBEND_PUBLIC_URL` | `server.public_url` |
| `WAYBEND_DATA_DIR` | Sets `storage.database` to `<value>/waybend.sqlite3` unless `WAYBEND_DATABASE` is also set. |
| `WAYBEND_DATABASE` | `storage.database` |
| `WAYBEND_DEFAULT_STATUS` | `redirects.default_status` |
| `WAYBEND_DEFAULT_HOPS` | `redirects.default_hops` |
| `WAYBEND_MAX_HOPS` | `redirects.max_hops` |
| `WAYBEND_PACKS` | `catalog.external_packs`, using the operating-system path separator |
| `WAYBEND_DNS_LISTEN` | Both `dns.udp_listen` and `dns.tcp_listen` |
| `WAYBEND_REBIND_DOMAIN` | `dns.domain`, and enables DNS when nonempty |
| `WAYBEND_ADMIN_TOKEN` | Overrides `security.admin_token` for the running HTTP service. |

`WAYBEND_LOG` controls log filtering, for example
`waybend=debug,tower_http=info`. `WAYBEND_LOG_FORMAT` selects `text` or `json`.
These are global CLI settings rather than YAML fields.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
