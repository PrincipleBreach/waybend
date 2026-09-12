# Waybend agent interface

## MCP

Start a local stdio server:

```sh
waybend mcp --config waybend.yml
```

Configure an MCP client to run that command with `WAYBEND_CONFIG` or an explicit `--config`. The server exposes `search_payloads`, `get_payload`, `build_redirect`, `dns_rebind_name`, and `list_evidence`.

## CLI fallback

```sh
waybend catalog --config waybend.yml --query metadata --format json
waybend encode --config waybend.yml --status 307 --hops 2 'gopher://127.0.0.1:6379/_PING%0D%0A'
waybend evidence --config waybend.yml list --token case-17 --json
```

Append `?token=case-17` to a catalog or dynamic URL. Waybend carries the query through every redirect hop and records it with redirect evidence.

## HTTP fallback

- `GET /api/v1/catalog` returns the complete compiled catalog.
- `GET /api/v1/encode?target=...&status=307&hops=2` builds a dynamic chain.
- `GET /api/v1/events?token=case-17&limit=100` returns correlated evidence.
- `GET /api/v1/capabilities` returns runtime hop limits and DNS availability.

Send `Authorization: Bearer <token>` when the instance has an admin token. Treat the `target` query parameter as ordinary URL-encoded form data; do not pre-normalize the terminal URI itself.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
