# Security Policy

## Reporting a Vulnerability

Email **security@principlebreach.com**. Do not open a public issue for an
unpublished vulnerability in Waybend or its release pipeline.

Include what you have:

- The output of `waybend version` and the platform or image digest.
- The smallest configuration and request that reproduce the issue.
- The impact and affected component.
- Any disclosure constraints and preferred credit.

Remove bearer tokens and captured request data that are not required for the
reproduction.

## Response Targets

| Stage | Target |
| --- | --- |
| Acknowledgment | 3 business days |
| Initial assessment | 10 business days |
| Fix or mitigation for a confirmed issue | 90 days |

We coordinate disclosure and credit with the reporter.

## Supported Versions

Security fixes land on the latest tagged release. Earlier releases do not
receive backports unless a release advisory says otherwise.

## Scope

In scope:

- Request handling, DNS parsing, redirect generation, and evidence storage.
- Authentication bypass for protected catalog, encoder, capability, or evidence APIs.
- Reads or writes outside configured storage and payload-pack paths.
- Release artifacts, signatures, attestations, containers, and update automation.

Waybend intentionally produces redirect targets for internal addresses and
non-HTTP schemes. A report needs an unintended behavior in Waybend itself, not
only a payload generated as designed.

## Deployment Boundary

Redirect and callback routes are public service surfaces. The bearer token, when
configured, protects catalog, encoder, capability, and evidence APIs. DNS does
not authenticate clients. Reverse proxies and network controls define who can
reach each surface.

Evidence can contain source addresses, tokens, headers, query strings, and
bounded bodies. Protect the SQLite database and backups to the same standard as
the captured requests.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
