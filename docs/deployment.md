# Deployment

## Container baseline

The production image is multi-stage and runs the release binary in Google's distroless `nonroot` image. Compose adds a read-only root filesystem, drops Linux capabilities, sets `no-new-privileges`, and uses a named volume for SQLite.

```console
cp waybend.example.yml waybend.yml
docker compose up -d
docker compose logs -f waybend
```

Pin `WAYBEND_IMAGE` to a release tag or digest in production.

The equivalent direct container run is:

```bash
docker run --rm \
  -p 8080:8080 \
  -p 5353:5353/tcp -p 5353:5353/udp \
  -v "$PWD/waybend.yml:/etc/waybend/config.yml:ro" \
  -v waybend-data:/var/lib/waybend \
  ghcr.io/principlebreach/waybend:latest
```

## HTTPS with Caddy

The optional Compose profile terminates HTTPS and forwards traffic to Waybend:

```console
export WAYBEND_HOSTNAME=bend.example.com
export WAYBEND_PUBLIC_URL=https://bend.example.com
docker compose --profile tls up -d
```

The hostname must resolve to the deployment and ports 80 and 443 must reach Caddy. Set `server.trust_proxy: true` only with direct access to the Waybend HTTP port restricted.

## DNS publishing

Compose publishes the configured container DNS socket as both TCP and UDP. Public authoritative DNS conventionally uses port 53:

```console
WAYBEND_DNS_PORT=53 docker compose up -d
```

Enable DNS and set the delegated domain in the YAML file. HTTPS proxying does not affect DNS.

## State and backup

SQLite and related files live in `/var/lib/waybend/data` in the container. Stop writes or use SQLite's online backup facilities when taking a consistent backup. Restore the database into a volume owned by the container's non-root user.

The retention task runs at startup and every six hours, removing events older than
`storage.retention_days`. Run `waybend evidence --config waybend.yml prune` for
an immediate prune.

## API Access

Set `security.admin_token` to protect the catalog, encoder, capabilities, and
evidence APIs. Redirects, callbacks, assets, and health endpoints remain public.
The web interface requests the bearer token when it receives an unauthorized
response and keeps it in browser session storage.

## Reverse proxies

The proxy should preserve `Host`, set `X-Forwarded-For`, and set `X-Forwarded-Proto`. It should not rewrite redirect paths or percent-encoded payload data. Configure request-size and timeout limits consistently with Waybend.

## Health and shutdown

Use `/health/live` for process liveness and `/health/ready` for readiness. Give the container runtime a normal termination grace period before forced shutdown.

Set `WAYBEND_LOG` to a tracing filter and `WAYBEND_LOG_FORMAT=json` for
structured container logs.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
