# Getting Started

## Install the Binary

Homebrew becomes available with the first tagged release:

```bash
brew install principlebreach/tap/waybend
```

Tagged releases publish binaries for Linux, macOS, and Windows. To build the
same binary from source, install Rust 1.88 or newer and run:

```bash
cargo build --release --locked
```

## Create a Configuration

Generate and validate a documented starter file:

```bash
waybend init waybend.yml
waybend validate --config waybend.yml
```

When running a source build, replace `waybend` with
`./target/release/waybend`. Set `server.public_url` to the base URL reachable by
the tested application. Set `storage.database` to a writable path.

## Start the Service

```bash
waybend serve --config waybend.yml
```

Confirm readiness and inspect one redirect without following its final target:

```bash
curl -fsS http://localhost:8080/health/ready
curl -sS -D - -o /dev/null \
  'http://localhost:8080/r/aws-ecs-v2/3?token=quickstart'
```

The second response contains:

```text
Location: http://169.254.170.2/v2/credentials/
```

Open `http://localhost:8080` to search the catalog. Each entry presents its
Waybend URL separately from the exact final target.

## Run with Docker Compose

```bash
cp waybend.example.yml waybend.yml
docker compose up --build -d
docker compose logs -f waybend
```

The `waybend-data` volume stores SQLite state. The configuration is mounted
read-only, the container root filesystem is read-only, and the process runs as
a non-root user.

## Correlate Evidence

Append one URL-safe token to a route or dynamic URL:

```text
http://localhost:8080/r/aws-ecs-v2?token=case-17
```

Waybend carries the token through intermediate hops and records redirect
observations under it. Callback URLs use `/c/<token>`.

```bash
waybend evidence --config waybend.yml list --token case-17 --json
```

## Add a Payload Pack

Create a YAML pack, add its path under `catalog.external_packs`, validate, and
restart the service. Pack loading is all-or-nothing.

```bash
waybend validate --config waybend.yml
waybend catalog --config waybend.yml --format csv
```

See [Payload packs](payload-packs.md) for the schema.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
