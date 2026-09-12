# Contributing to Waybend

Waybend is built by [Principle Breach](https://principlebreach.com).
Contributions are welcome.

## Ground Rules

- New behavior ships with focused tests. A bug fix reproduces the failure first.
- Payloads represent exact, documented parser or protocol behavior.
- Invalid configuration fails closed; unknown fields never become silent defaults.
- Redirect targets retain their original bytes, casing, and percent encoding.
- Public behavior changes include the matching configuration and documentation changes.

## Local Development

Install Rust 1.88 or newer and `cargo-audit`, then run:

```bash
make check
make audit
```

`make check` enforces formatting, Clippy with warnings denied, and all tests.
Build and run the example configuration with:

```bash
make build
make run
```

## Add a Payload

Use an external [payload pack](docs/payload-packs.md) when the route is specific
to one environment. A built-in route belongs in `src/catalog.rs` only when its
behavior is reusable and traceable to a primary protocol or platform source.

For a built-in route:

1. Add the smallest deterministic generator or definition.
2. Add an exact-output test, including encoded or binary selector bytes.
3. Label client-dependent behavior in its tags and documentation.
4. Run `make check` and `make audit`.

## Pull Requests

Keep each pull request centered on one outcome. Include the problem, observable
behavior before and after, and the commands used to verify it. Do not commit
build output, SQLite databases, credentials, or captured request data.

Use Conventional Commits:

```text
feat(dns): add deterministic TOCTOU answers
fix(config): reject zero redirect hops
docs: document reverse proxy identity
```

## Reporting Issues

Include the Waybend version, platform or container tag, redacted configuration,
exact reproduction, expected result, and actual result. Report vulnerabilities
through [SECURITY.md](SECURITY.md).

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
