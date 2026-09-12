# Payload packs

Payload packs are versioned YAML documents loaded at startup. Unknown fields, duplicate IDs, invalid headers, unsupported redirect statuses, and invalid hop counts fail validation.

```yaml
name: team-routes
version: "1.0.0"
description: Team-specific request targets
routes:
  - id: internal-admin
    title: Internal admin over HTTP
    category: internal
    description: Redirects to the conventional internal admin endpoint
    tags: [http, internal]
    target: http://127.0.0.1:8080/admin?source={{token}}
    status: 302
    hops: 3
    headers:
      X-Waybend-Client: "{{client_ip}}"
    enabled: true
```

## Route fields

| Field | Required | Meaning |
|---|---:|---|
| `id` | yes | Stable lowercase route identifier. IDs are unique across every pack. |
| `title`, `category`, `target` | yes | Display metadata and exact final redirect target. |
| `description`, `tags` | no | Searchable catalog metadata. |
| `status` | no | Redirect status; defaults to 302. |
| `hops` | no | Redirect count; defaults to 3 and must not exceed `redirects.max_hops`. |
| `headers` | no | Additional response headers with template support. |
| `enabled` | no | Set false to retain a definition without publishing it. |

Targets and header values support `{{client_ip}}`, `{{referer_host}}`, `{{token}}`, and `{{public_url}}`. Waybend preserves target spelling and encoding, including non-HTTP schemes.

## Load packs

Add a file or directory to the ordered list:

```yaml
catalog:
  include_builtin: true
  external_packs:
    - /etc/waybend/packs
```

Directories load `.yml` and `.yaml` files in sorted order. Validate before restart:

```console
waybend validate --config waybend.yml
waybend catalog --config waybend.yml
```

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
