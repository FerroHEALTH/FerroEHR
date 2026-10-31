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
| `issuer-metadata.json` | The RFC 8414 §2 metadata of the test issuer, mounted as `smart.issuer_metadata_file`, so the SUT's discovery document names a `token_endpoint`. No authorization server answers at its endpoints. |

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
   document is on, built from `issuer-metadata.json`, because a key-set file
   carries no metadata of its own.
2. `docs/conformance/party/ferroehr/ixit.json` declares the issuer the
   instrument mints with: `scoped_tokens.mint` names the issuer, audience,
   subject and `kid` of the RS256 token Veredictum signs for each scope set a
   `smart_scopes` row presents. It names no token and no key: each token is
   minted in memory for the run, with the key handed over as
   `VEREDICTUM_SCOPED_TOKEN_KEY_FILE`.
3. `scripts/conformance.sh` composes the overlay on every FerroEHR run and
   drives the `smart_discovery` and `smart_scopes` documents in the same
   `veredictum test` as the rest of the catalogue, with `--platform-url` and
   that variable pointing at this key. The conformance workflow does the same
   through the Veredictum Action's `platform-url` input and the
   `VEREDICTUM_SCOPED_TOKEN_KEY_FILE` variable on the step (#4503).

Spec: `docs/specs/openehr/ITS-REST/docs/smart_app_launch/` —
`master04-service_discovery.adoc` (the discovery document),
`master08-scopes.adoc` (the resource-scope grammar),
`master06-authentication.adoc` (the CDR is a resource server: it validates
tokens and never issues them, which is why the *runner* mints here).
