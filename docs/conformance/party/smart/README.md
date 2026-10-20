# CNF SMART lane — the static test issuer

## THIS IS COMMITTED TEST KEY MATERIAL. IT IS NOT SECRET AND MUST NEVER BE USED IN PRODUCTION.

`cnf-smart-test.key.pem` is a **passphrase-less RSA-2048 private key that lives
in public version control**. Anyone with a checkout can mint tokens with it. It
exists for exactly one purpose: to let the CNF runner act as the
Authorization Server the SMART conformance lane needs, without the conformance
stack having to run one.

Never configure a deployment that holds real data to trust this issuer. If a
deployment ever does, every token it accepts is forgeable by anyone.

## What is here

| File | What it is |
|---|---|
| `cnf-smart-test.key.pem` | The RSA-2048 private key (PKCS#8 PEM) the runner signs RS256 access tokens with. |
| `jwks.json` | The matching **public** JWKS (`kid: cnf-smart-test`), mounted into the SUT as `auth.jwks_file`. |

The pair is generated once and committed, so a conformance run is reproducible
and needs no key-generation step. It was produced with:

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
  -out docs/conformance/party/smart/cnf-smart-test.key.pem
# jwks.json is the public half: kty=RSA, kid=cnf-smart-test, alg=RS256,
# n = the modulus (base64url), e = AQAB.
```

## How the lane wires it together

1. `docker/sut-smart.yml` (the compose overlay) mounts `jwks.json` into the
   SUT and points `[auth]` at it (`auth.jwks_file`), with a static issuer and
   audience and the `jwt` token profile the runner's tokens meet. Every route
   enforces its SMART on openEHR requirement with no switch; the discovery
   document stays off in this lane, because it is built from the issuer's
   metadata and a key-set file carries none.
2. `docs/conformance/party/ferroehr/ixit.json` declares the posture: the
   `smart` block names this key file, the issuer, the audience and the `kid`;
   the `sut`/`admin`/`readonly` principals carry `bearer_mint` auth with
   per-instance roles + standing scope grants, and the `smart_app` instance
   mints exactly the step-declared `scopes` for the boundary cases.
3. `scripts/conformance.sh` composes the posture on every FerroEHR run —
   it IS the standard conformance posture (owner ruling 2026-07-28), so the
   one committed record covers the whole claimed surface in one invocation.

Cases that declare `scopes:` on a flow step — and every case anchored on the
`I_ITS_REST_SMART` pseudo-interface — are **not-applicable** when a party's
ixit declares no `smart` block (ISO/IEC 9646 test selection): a SUT that
does not claim the capability is never driven against a guess (ehrbase).

Spec: `docs/specs/openehr/ITS-REST/docs/smart_app_launch/` —
`master04-service_discovery.adoc` (the discovery document),
`master08-scopes.adoc` (the resource-scope grammar),
`master06-authentication.adoc` (the CDR is a resource server: it validates
tokens and never issues them, which is why the *runner* mints here).
