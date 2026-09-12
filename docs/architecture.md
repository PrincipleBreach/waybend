# Architecture

Waybend is a single Rust process with independently testable core behavior and thin network adapters.

```text
configuration + payload packs
             │
             ▼
      validated catalog ───────► HTML/JSON catalog ─► MCP tools
             │
             ▼
       redirect engine ────────► HTTP redirect chains

HTTP callbacks ─┐
DNS evidence ───┴──────────────► SQLite evidence store ─► evidence API
                                      │
                                      └─────────────────► MCP tools

DNS packets ───────────────────► pure packet handler ───► authoritative reply
```

## Ownership

- `config` parses environment/YAML and owns whole-application validation.
- `catalog` loads built-in and external route definitions and owns catalog uniqueness and filters.
- `engine` renders templates and owns redirect-hop behavior.
- `server` maps HTTP requests to catalog, engine, and storage operations.
- `dns` parses and emits DNS wire messages; the stateful listener only owns per-name sequence counters.
- `storage` owns the SQLite schema and evidence queries.
- `mcp` maps bounded tool inputs to catalog, encoder, DNS-name, and evidence operations.
- `cli` loads shared state and starts the HTTP/DNS or stdio MCP adapter.

DNS's packet handler accepts its sequence number and evidence hook as inputs. Unit tests can therefore prove exact answers without sockets or timing. The listener supplies monotonic per-name counts and supports UDP and TCP concurrently.

## Data lifecycle

Configuration and packs are loaded at startup. Redirects and MCP searches read
immutable catalog data. Callback, redirect, and DNS requests append evidence to
SQLite. HTTP and MCP queries read that evidence with bounded result counts. The
server prunes expired rows at startup and every six hours; the CLI can trigger
the same operation immediately.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
