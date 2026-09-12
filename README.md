<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/banner-dark.svg">
  <source media="(prefers-color-scheme: light)" srcset=".github/banner-light.svg">
  <img alt="Principle Breach" src=".github/banner-dark.svg" width="100%">
</picture>

<p align="center">
  <img alt="Waybend logo" src=".github/waybend-mark.svg" width="132" height="132">
</p>

# Waybend

Self-hosted SSRF infrastructure for redirect chains, exact protocol payloads,
callbacks, DNS rebinding, and request evidence. One Rust binary provides the
web workbench, JSON API, authoritative DNS responder, and stdio MCP interface.

<!-- ------------------------------------------------------------ -->

## Install

Homebrew becomes available with the first tagged release:

```bash
brew install principlebreach/tap/waybend
```

Tagged releases also publish signed binaries for Linux, macOS, and Windows on
the [releases page](https://github.com/PrincipleBreach/waybend/releases), plus a
multi-platform container at `ghcr.io/principlebreach/waybend`.

Build from source with Rust 1.88 or newer:

```bash
git clone https://github.com/PrincipleBreach/waybend.git
cd waybend
cargo build --release --locked
```

## Start

```bash
waybend init waybend.yml
waybend validate --config waybend.yml
waybend serve --config waybend.yml
```

Open `http://localhost:8080`. Each catalog entry shows both the copy-ready
Waybend URL and its exact final target:

```text
Waybend URL   http://localhost:8080/r/aws-ecs-v2?token=case-17
Final target http://169.254.170.2/v2/credentials/
```

Use Docker Compose for a persistent local deployment:

```bash
cp waybend.example.yml waybend.yml
docker compose up --build -d
```

See [Getting started](docs/getting-started.md) for containers, native binaries,
validation, and the first redirect test.

## What It Does

- Runs deterministic 1–10 hop chains with 301, 302, 303, 307, or 308 responses.
- Preserves exact terminal targets, including non-HTTP schemes and encoded bytes.
- Ships 562 built-in routes; configuring authoritative DNS adds 16 rebind routes.
- Captures HTTP callbacks, redirect observations, and DNS answers in SQLite.
- Loads strict YAML payload packs without recompiling the binary.
- Gives agents five typed MCP tools through a portable Waybend skill.

The catalog covers cloud metadata, loopback and private-network address forms,
URL parser differentials, local-file targets, and protocol-framed Gopher routes.
The protocol boundary and primary references are documented in
[Protocol payloads](docs/protocols.md).

## Use It

Search or export the catalog without starting a server:

```bash
waybend catalog --config waybend.yml --query metadata
waybend catalog --config waybend.yml --category internal-service --format csv
```

Build a byte-preserving dynamic redirect:

```bash
waybend encode --config waybend.yml --status 307 --hops 2 \
  'Gopher://127.0.0.1:6379/_INFO%0D%0APING'
```

Correlate a route with `?token=<value>`, then query its observations:

```bash
waybend evidence --config waybend.yml list --token case-17 --json
```

Run `waybend --help` or `waybend <command> --help` for the installed command
surface.

## Agent Integration

Start the standard-input/output MCP server:

```bash
waybend mcp --config /absolute/path/to/waybend.yml
```

It exposes `search_payloads`, `get_payload`, `build_redirect`,
`dns_rebind_name`, and `list_evidence`. The
[Waybend skill](skills/waybend/SKILL.md) directs an agent to select a focused
payload set, preserve protocol bytes, reuse one correlation token, and report
observed evidence.

See [Agent integration](docs/agent-integration.md) for MCP client configuration,
the tool schemas, and skill installation.

## DNS Rebinding

Delegate a zone to Waybend, enable DNS, and publish both UDP and TCP port 53.
Query names encode static, alternating, or deterministic TOCTOU behavior:

```text
127-0-0-1.static.rb.example.com
198-51-100-10.127-0-0-1.alt.rb.example.com
198-51-100-10.169-254-169-254.2.toctou.rb.example.com
```

Prefix a name with `t-<token>.` to join DNS and HTTP evidence. See
[DNS rebinding](docs/dns.md) for delegation, IPv6 encoding, and sequence rules.

## Documentation

- [Configuration](docs/configuration.md) — every YAML field and environment override.
- [HTTP API](docs/api.md) — redirects, callbacks, evidence, health, and errors.
- [Payload packs](docs/payload-packs.md) — add exact targets and response headers.
- [Deployment](docs/deployment.md) — container, TLS proxy, DNS, storage, and logs.
- [Architecture](docs/architecture.md) — process boundaries and data lifecycle.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities through
[SECURITY.md](SECURITY.md).

## License

MIT — see [LICENSE](LICENSE).

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
