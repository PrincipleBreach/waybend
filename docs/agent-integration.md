# Agent Integration

Waybend exposes the same catalog and evidence through a typed Model Context
Protocol server. The MCP process uses standard input/output; the HTTP service
continues to execute redirect and callback requests.

## Start the MCP Server

Use an absolute configuration path so the client does not depend on its working
directory:

```bash
waybend mcp --config /absolute/path/to/waybend.yml
```

Generic MCP client configuration:

```json
{
  "mcpServers": {
    "waybend": {
      "command": "waybend",
      "args": ["mcp", "--config", "/absolute/path/to/waybend.yml"]
    }
  }
}
```

The configuration's `server.public_url` must point to the Waybend HTTP service
that the tested application can reach. MCP builds URLs; HTTP and DNS perform the
network behavior and record evidence.

## Tools

| Tool | Purpose | Important input |
| --- | --- | --- |
| `search_payloads` | Search route metadata and exact targets. | `query`, `category`, `tags`, `limit` |
| `get_payload` | Resolve one route into its Waybend URL and final target. | `id`, optional `token` |
| `build_redirect` | Encode any exact terminal URI into a redirect chain. | `target`, optional `status`, `hops`, `token` |
| `dns_rebind_name` | Build a static, alternating, or TOCTOU hostname. | `mode`, optional addresses, token, switch count |
| `list_evidence` | Read observed redirect, callback, and DNS events. | token, kind, route ID, search, limit |

Search returns at most 100 routes. Evidence returns at most 200 events. Tokens
are 1–128 characters from letters, digits, `-`, `_`, `.`, or `~`.

## Install the Skill

The portable skill lives at [`skills/waybend`](../skills/waybend). Copy that
directory into the MCP client's supported skill directory, or expose it as a
repository-local skill. Keep `SKILL.md`, `agents/openai.yaml`, and
`references/interface.md` together.

The skill defines a bounded workflow:

1. Select routes for one parser, protocol, or address hypothesis.
2. Reuse one token across the redirect, callback, and DNS names.
3. Preserve Gopher selector bytes and other exact targets.
4. Execute the returned URL through the test interface.
5. Query evidence and report what Waybend observed.

## HTTP and CLI Fallbacks

An agent without MCP can use the JSON API:

```text
GET /api/v1/catalog
GET /api/v1/encode?target=<url-encoded-target>&status=307&hops=2
GET /api/v1/events?token=case-17&limit=100
GET /api/v1/capabilities
```

Or use the CLI:

```bash
waybend catalog --config waybend.yml --query metadata --format json
waybend encode --config waybend.yml --status 307 --hops 2 '<target>'
waybend evidence --config waybend.yml list --token case-17 --json
```

Protected HTTP APIs require `Authorization: Bearer <admin_token>`. The local MCP
process reads the catalog and evidence database directly from its configuration.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
