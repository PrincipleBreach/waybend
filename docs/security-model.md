# Security model

## Trust boundaries

Waybend receives untrusted HTTP requests, callback bodies, DNS packets, payload-pack files, and proxy headers. Configuration files, external packs, the SQLite volume, and administrators are trusted deployment inputs.

The service intentionally generates arbitrary redirect targets, including
non-HTTP schemes and internal address ranges. `security.admin_token` or
`WAYBEND_ADMIN_TOKEN` protects catalog, encoder, capabilities, and evidence APIs.
Redirects, callbacks, assets, and health remain public service surfaces.

## Enforced bounds

- Configuration rejects unknown fields and invalid statuses, URLs, domains, and hop counts.
- HTTP and retained callback bodies have independent byte limits.
- Evidence list queries are bounded.
- DNS parsing uses the protocol library, refuses out-of-zone questions, and emits answers only for the configured authoritative zone.
- The container runs as a non-root user with a read-only root filesystem in Compose.

## Sensitive data

Evidence can include source addresses, headers, tokens, paths, queries, and body fragments. SQLite files and backups have the same sensitivity as the captured requests. Logs should use metadata rather than request bodies or authorization values.

## Reverse proxy identity

With `trust_proxy: false`, Waybend uses its direct peer. With it enabled, the reverse proxy defines client identity and external scheme/host. Network policy should prevent clients from bypassing that trusted proxy.

## Reporting

Follow [the security policy](../SECURITY.md) for private vulnerability reports.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
