# Protocol payloads

Waybend's built-in service routes are generated from wire formats, not copied payload strings. Each Gopher target percent-encodes every selector octet so an intermediary cannot reinterpret spaces, delimiters, or binary bytes.

## Compatibility classes

| Tags | What the target requires |
| --- | --- |
| `single-line` | A Gopher client decodes the selector and appends its mandatory CRLF. No CR, LF, or NUL is embedded in the URL. |
| `binary-selector`, `client-dependent` | A client decodes every percent-encoded octet, including CR, LF, and NUL, before appending the Gopher CRLF. Test the target application's URL client before relying on this class. |

RFC 4266 defines selectors as excluding CR and LF. Current curl additionally rejects a decoded CR, LF, or NUL in a Gopher selector. Consequently, current curl can carry the `single-line` routes but intentionally rejects the `binary-selector` routes. Waybend labels this boundary in the catalog instead of presenting binary selectors as universally portable.

## Generated protocols

### Redis

The `gopher-redis-line-*` routes use Redis's inline-command compatibility syntax. The selector omits CRLF because the Gopher transport supplies it. The `gopher-redis-resp-*` routes generate RESP2 arrays of bulk strings, including byte lengths and internal CRLF separators. They omit only the final CRLF, which the Gopher transport supplies, so the bytes placed on the connection form one complete RESP command.

### Memcached

The ASCII protocol routes emit `version`, `stats`, `stats items`, or `stats slabs`. Each is a single command line; Gopher supplies its CRLF terminator. No storage or mutation command is included in the built-in pack.

### ZooKeeper

The routes emit exactly one four-letter word: `ruok`, `stat`, `envi`, or `conf`. ZooKeeper 3.5.3 and later requires these commands to be explicitly allowed by `4lw.commands.whitelist`. A disabled command can still produce a rejection response, depending on the server version.

### SMTP

The routes encode RFC 5321 commands with valid argument forms: `EHLO waybend.invalid`, `NOOP`, `HELP`, and `VRFY root`. The Gopher terminator supplies each command's CRLF. An SMTP server normally sends its `220` greeting before it reads the command; clients that only write and then read can still receive both the greeting and command response.

### FastCGI

Each FastCGI target contains a complete responder request with request ID 1:

1. `FCGI_BEGIN_REQUEST` with role `FCGI_RESPONDER` and `FCGI_KEEP_CONN` clear.
2. One `FCGI_PARAMS` record containing CGI name-value pairs.
3. An empty `FCGI_PARAMS` record terminating that stream.
4. An empty `FCGI_STDIN` record terminating input.

Names and values below 128 bytes use the specification's one-byte length form. The final STDIN record declares two padding bytes; Gopher's trailing CRLF fills those ignored padding octets, leaving no extra unframed bytes. The built-in script root is `/var/www/html`; use an external pack for a different deployment layout.

### Zabbix agent

Zabbix agent routes target the passive-agent port `10050`, not the sender/server port `10051`. They generate the current JSON passive-check request for one item key, prefixed with `ZBXD`, flags `0x01`, a 32-bit little-endian data length, and four zero reserved bytes. The declared data length includes Gopher's trailing CRLF, which is valid trailing JSON whitespace and therefore remains inside the Zabbix frame.

### DICT

DICT URLs are protocol operations, not arbitrary TCP payload carriers. Waybend exposes only RFC 2229 URL forms: `/d:` for `DEFINE` and `/m:` for `MATCH`, both on port 2628. It does not label DICT URLs aimed at Redis, SMTP, or other ports as raw-command payloads because a conforming DICT client constructs DICT commands from the URL operation.

## Redirect headers

Built-in targets do not attach metadata or forwarding headers. Headers in a Waybend route are fields on Waybend's redirect response; HTTP redirect semantics do not copy those fields into the client's next request. External packs can still set response headers for redirect-client research, but the catalog does not describe them as target request headers.

## URL parser variants

The localhost matrix includes decimal dword, hexadecimal, octal, mixed-radix, shortened two- and three-part IPv4, padded components, trailing-dot hosts, IPv4-mapped IPv6, and IPv4-compatible IPv6. The parser-differential category adds userinfo, explicit-port, query, and fragment boundaries. These variants are deliberately separate because RFC 3986 URI parsing, WHATWG URL parsing, and operating-system address conversion do not accept or normalize exactly the same input set.

## DNS rebind targets

Every built-in rebind URL uses a hostname implemented by Waybend's authoritative DNS server:

- `alt.<zone>` alternates configured first and second addresses.
- `2.toctou.<zone>` returns the first address twice, then the second address.

The catalog produces both patterns for each HTTP destination. Token correlation can be added by prefixing either hostname with `t-<token>.`.

## Primary references

- [Redis serialization protocol](https://redis.io/docs/latest/develop/reference/protocol-spec/)
- [memcached text protocol](https://github.com/memcached/memcached/blob/master/doc/protocol.txt)
- [ZooKeeper four-letter commands](https://zookeeper.apache.org/doc/r3.9.2/zookeeperAdmin.html#sc_zkCommands)
- [SMTP, RFC 5321](https://www.rfc-editor.org/rfc/rfc5321)
- [FastCGI specification](https://fastcgi-archives.github.io/FastCGI_Specification.html)
- [Zabbix passive checks](https://www.zabbix.com/documentation/current/en/manual/appendix/items/activepassive)
- [Zabbix protocol header](https://www.zabbix.com/documentation/current/en/manual/appendix/protocols/header_datalen)
- [DICT, RFC 2229](https://www.rfc-editor.org/rfc/rfc2229)
- [Gopher URI scheme, RFC 4266](https://www.rfc-editor.org/rfc/rfc4266)
- [curl Gopher implementation](https://github.com/curl/curl/blob/master/lib/gopher.c)
- [URI generic syntax, RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
- [WHATWG URL Standard](https://url.spec.whatwg.org/)

---

<sub>Built by <a href="https://principlebreach.com">Principle Breach</a> — offensive security research.</sub>
