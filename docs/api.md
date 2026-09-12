# HTTP API

All JSON API responses use `application/json`. If `security.admin_token` or
`WAYBEND_ADMIN_TOKEN` is set, catalog, encoder, capabilities, and evidence endpoints require
`Authorization: Bearer <token>`. Redirect, callback, asset, and health endpoints
remain public.

## Catalog

`GET /api/v1/catalog` and `GET /api/v1/routes` return the same catalog envelope:

```json
{
  "routes": [
    {
      "id": "aws-imds-iam",
      "title": "AWS IAM role list",
      "category": "cloud-metadata",
      "description": "Redirects to AWS IAM role list.",
      "tags": ["cloud", "aws", "metadata"],
      "target": "http://169.254.169.254/latest/meta-data/iam/security-credentials/",
      "status": 302,
      "hops": 3,
      "headers": {},
      "source": "builtin:waybend-0.1"
    }
  ],
  "count": 1
}
```

## Redirect routes

| Request | Behavior |
|---|---|
| `ANY /r/{id}` | Start route `{id}` at hop 1. |
| `ANY /r/{id}/{hop}` | Execute a specific numbered hop. |

Redirect responses have the route status, `Location`, `Cache-Control: no-store`, and configured route headers. Add `?token=<value>` to correlate redirect evidence and template output. Tokens are 1–128 URL-safe ASCII characters using letters, digits, `-`, `_`, `.`, or `~`. Without one, Waybend creates a random token. Waybend carries the complete query and stable token across every intermediate hop.

When `redirects.preserve_query` is enabled, Waybend appends the inbound query string to a catalog route's final target.

## Dynamic redirects

When `redirects.allow_target_override` is enabled, any exact target can be encoded into a path:

| Request | Behavior |
|---|---|
| `ANY /d/{status}/{hops}/{base64url}` | Start a dynamic redirect chain. |
| `ANY /d/{status}/{hops}/{base64url}/{hop}` | Execute a numbered dynamic hop. |
| `GET /api/v1/encode?target=...&status=302&hops=3` | Return a copy-ready dynamic URL and encoded value. |

The target uses unpadded URL-safe Base64. It is rendered only at the final hop
without URL normalization. Template variables work in dynamic targets. The
encoder response contains `url`, `encoded`, `status`, and `hops`.

## HTTP callbacks

`ANY /c/{token}` and `ANY /c/{token}/{path...}` capture the request and return:

```json
{"captured":true,"event_id":"550e8400-e29b-41d4-a716-446655440000"}
```

Methods, path, query, headers, source address, and a bounded body are stored. Binary header values and bodies are represented as `base64:<data>`.
Repeated request headers are represented as arrays without collapsing values. DNS events include `dns_answer` and the zero-based `dns_sequence` that selected it.

## Capabilities

`GET /api/v1/capabilities` returns the running version, default redirect status, default and maximum hop counts, dynamic-redirect availability, and authoritative DNS domain. The web UI reads this endpoint instead of assuming configuration defaults.

## Evidence

`GET /api/v1/events` accepts these query parameters:

| Parameter | Meaning |
|---|---|
| `limit` | Page size; storage caps it at 500. Default: 100. |
| `offset` | Zero-based result offset. |
| `token` | Exact token filter. |
| `kind` | Exact event kind, including `http`, `redirect`, or `dns`. |
| `route_id` | Exact redirect route ID. |
| `q` | Text search across indexed event content. |

The response contains `events`, the returned page `count`, and `total` for the
active filters before limit and offset. `GET /api/v1/events/{id}` returns one
event or a 404 error envelope.

## Health

| Request | Success |
|---|---|
| `GET /health/live` | Process is serving HTTP. |
| `GET /health/ready` | Evidence storage health check succeeded. |

Both return a status and build version. Readiness returns HTTP 503 when storage is unavailable.

## Errors

API failures use an HTTP status plus a stable code and message:

```json
{
  "error": {
    "code": "route_not_found",
    "message": "redirect route \"missing\" does not exist"
  }
}
```

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
