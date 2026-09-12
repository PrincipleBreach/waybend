# CLAUDE.md

Guidance for coding agents working in this repository.

## What This Is

Waybend is a self-hosted SSRF redirect, payload, callback, and authoritative DNS
workbench. It is one Rust binary with an embedded web UI, JSON API, SQLite
evidence store, stdio MCP server, and no runtime dependency on a hosted service.

## Build, Test, Run

```bash
make check       # format check, Clippy with warnings denied, all tests
make audit       # dependency advisory scan
make build       # debug binary
make run         # serve waybend.example.yml
make docker-build
```

Before a commit, `make check` and `make audit` are clean. For network changes,
also run the binary and exercise the changed HTTP or DNS path over a real socket.

## Layout

```text
src/
  catalog.rs     built-in routes, pack loading, catalog filters
  config.rs      strict YAML, environment overrides, validation
  engine.rs      templates and redirect-hop construction
  server.rs      HTTP/UI/API adapters
  dns.rs         authoritative UDP/TCP DNS and answer sequencing
  storage.rs     SQLite schema, evidence queries, retention
  mcp.rs         typed Model Context Protocol tools
  cli.rs         command tree and process orchestration
ui/              embedded HTML, CSS, and JavaScript
packs/           external-pack example
skills/waybend/  portable agent skill and interface reference
docs/            operator and protocol documentation
```

## Extension Points

Environment-specific targets belong in YAML payload packs. Built-in payloads
belong in `src/catalog.rs` only when their behavior is reusable, deterministic,
and backed by a primary source. MCP tools live in `src/mcp.rs`; keep their input
and output schemas bounded and stable.

## Non-Negotiable Behavior

1. Preserve target bytes, scheme casing, and encoded protocol selectors.
2. Reject unknown or invalid configuration instead of selecting a fallback.
3. Keep packet parsing, route rendering, and validation deterministic and tested.
4. Bound bodies, result counts, hop counts, and retained evidence.
5. Keep network handlers thin; put protocol behavior in pure functions.

The binary makes no telemetry call. Do not add a hosted dependency to core
operation.

## Documentation and Commits

- Use terse, declarative American English. No emoji or hype.
- Every Markdown file ends with the Principle Breach footer below.
- Keep internal planning, roadmaps, and launch strategy out of the repository.
- Use Conventional Commits and one logical change per commit.
- Never add agent or tool attribution to commits or pull requests.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
