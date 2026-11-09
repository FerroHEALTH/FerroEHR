# Version signing

Every version FerroEHR commits can carry a `VERSION.signature`: a value
computed over the version's own canonical form, inside the write transaction
that stores it. openEHR defines two depths for that value, and FerroEHR
implements both as first-class modes ([RM Common IM, Change Control §Digital
Signature](https://specifications.openehr.org/releases/RM/Release-1.1.0/common.html#_digital_signature)).

| Mode                   | What is stored                               | What it establishes                              |
|------------------------|----------------------------------------------|--------------------------------------------------|
| `digest` (the default) | `sha256:` + base64(SHA-256(canonical form))  | the version has not been altered since committal |
| `pgp`                  | an ASCII-armored RFC 4880 detached signature | that, plus which key signed it                   |

Signing is **on by default**, in `digest` mode, and read-time verification of
the server's own signatures defaults to `strict`: a served version that no
longer matches its stored signature is a `500` rather than a silently served
record.

## The two pages

- **[Digest signing](digest.md)** covers the mechanism both modes share: what
  goes into the signed form, when it is computed, what a stored digest does and
  does not prove, how verification at read behaves, and how to reproduce a
  digest yourself from the served JSON.
- **[PGP signing](pgp.md)** covers what changes when a key is involved: the key
  configuration and its fail-closed boot check, rotation and retired keys,
  client-supplied signatures versus server-generated ones, and the signature an
  import wrapper carries.

Read them in that order; the PGP page builds on the digest page rather than
repeating it.

## Where this sits

Version signing is openEHR's own record-level integrity mechanism, and it is
distinct from the two other integrity surfaces this book describes. The
[ATNA audit trail](../audit.md) records security surveillance of API access,
with its own hash chain over audit records.
[Verifying releases](../verifying-releases.md) is about the artifacts you
downloaded, not the data you stored. All three coexist and none substitutes
for another.

Every `[record.signing]` key, with defaults and environment forms, is in the
[configuration reference](../installation/config-auth.md#signing).
