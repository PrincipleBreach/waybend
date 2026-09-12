---
name: waybend
description: Use a self-hosted Waybend instance to select SSRF bypass payloads, build exact redirect chains or rebinding names, and correlate HTTP/DNS evidence. Apply when an authorized bug-bounty or security-testing task needs SSRF payload generation or Waybend evidence analysis.
---

# Waybend

Use the Waybend MCP tools when connected. If only the binary or HTTP endpoint is available, read [references/interface.md](references/interface.md).

1. Derive a short URL-safe correlation token from the current test case. Reuse it for every request in that case.
2. Call `search_payloads` with the narrowest technique, category, and tags that match the target behavior. Inspect exact targets with `get_payload`.
3. Use `build_redirect` when the catalog has no exact terminal URI. Preserve scheme casing and percent-encoded bytes; do not decode and re-encode Gopher selectors.
4. Use `dns_rebind_name` for resolver/parser differentials. Choose `alt` for alternating answers or `toctou` when the expected validation and connection lookup counts are known.
5. Execute only through the testing interface and target scope supplied by the user. Call `list_evidence` with the same token and report observed requests, DNS answers, and sequence numbers rather than inferring success from an emitted payload.

Prefer a small hypothesis-driven set over dumping the entire catalog. Change one dimension at a time—address spelling, URL syntax, redirect status/hops, protocol framing, or DNS answer sequence—so evidence identifies what bypassed the target parser or validator.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
