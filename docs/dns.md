# DNS rebinding

Waybend contains an authoritative DNS responder over UDP and TCP. Answers are generated from query names and a per-name sequence counter; no upstream resolver is involved.

## Delegate the zone

For a zone such as `rb.example.com`:

1. Create `A` and `AAAA` records for `ns1.rb.example.com` that point to the Waybend host.
2. Add an `NS` delegation for `rb.example.com` to `ns1.rb.example.com` at the parent DNS provider.
3. Forward public TCP and UDP port 53 to Waybend's configured DNS socket.
4. Set `dns.enabled`, `dns.domain`, and a low nonzero `dns.ttl`.

Test both transports:

```console
dig @203.0.113.10 127-0-0-1.static.rb.example.com A
dig +tcp @203.0.113.10 127-0-0-1.static.rb.example.com A
```

## Name formats

IPv4 labels replace dots with dashes. `169.254.169.254` becomes `169-254-169-254`.

IPv6 labels use exactly 32 hexadecimal digits with separators removed. `::1` becomes `00000000000000000000000000000001`.

| Pattern | Sequence |
|---|---|
| `static.<zone>` | Always `dns.first_ip`. |
| `alt.<zone>` | Alternate between `dns.first_ip` and `dns.second_ip`. |
| `<count>.toctou.<zone>` | `dns.first_ip` for `count` queries, then `dns.second_ip`. |
| `<ip>.static.<zone>` | Always `<ip>`. |
| `<first>.<second>.alt.<zone>` | `<first>`, `<second>`, then repeat. |
| `<first>.<second>.<count>.toctou.<zone>` | `<first>` for `count` queries, then `<second>` permanently. |

Prefix any pattern with `t-<token>.` to correlate it in the evidence store. For example, `t-case-7.198-51-100-10.127-0-0-1.alt.rb.example.com` alternates normally and records `case-7` as its token.

The sequence is zero-based and maintained independently per fully qualified
query name and record type. A and AAAA queries only return an answer when the
selected address has the matching family. Other record types receive an
authoritative empty answer. Invalid in-zone names receive `NXDOMAIN`;
out-of-zone names receive `REFUSED`.

## Evidence

Every syntactically valid in-zone query emits a DNS evidence event containing source socket, query name, record type, selected answer, and sequence number. The server integration maps this into the shared evidence store so DNS and HTTP observations can be searched together.

## Operational details

- UDP packets accept the DNS maximum wire size; TCP uses the standard two-byte message length prefix and supports multiple requests per connection.
- Responses copy the request ID and question, set the authoritative bit, and do not offer recursion.
- Counters are process-local. Restarting Waybend resets every name to sequence zero.
- Run one authoritative Waybend instance per zone when exact sequence order matters. Independent replicas maintain independent counters.

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
