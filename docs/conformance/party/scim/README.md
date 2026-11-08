# The development lanes' SCIM provisioning client

## THIS IS COMMITTED TEST KEY MATERIAL. IT IS NOT SECRET AND MUST NEVER BE USED IN PRODUCTION.

`scim-test.key.pem` is a passphrase-less RSA-2048 private key in public
version control. Anyone with a checkout can mint a SCIM client token with
it. It exists for one purpose: the development lanes provision the persons
they log in as through `/scim/v2/Users` before their run, and FerroEHR issues
no tokens, so the lanes sign their own.

Never configure a deployment that holds real data to trust this issuer. If a
deployment does, anyone can create and delete its professionals.

## What is here

| File | What it is |
|---|---|
| `scim-test.key.pem` | The RSA-2048 private key (PKCS#8 PEM) `scripts/scim-provision.sh` signs RS256 `at+jwt` access tokens with. |
| `jwks.json` | The matching public JWKS (`kid: scim-test`), mounted into the server as the `jwks_file` of the `machines` issuer `https://scim.ferroehr.invalid`. |

The pair was generated once with:

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 \
  -out docs/conformance/party/scim/scim-test.key.pem
# jwks.json is the public half: kty=RSA, kid=scim-test, alg=RS256,
# n = the modulus (base64url), e = AQAB.
```

## How the lanes use it

`docker/ferroehr.dev.toml` trusts the issuer (audience `ferroehr-scim`) and
declares the SCIM client `basic-users`, whose `binding_issuer` is
`urn:ferroehr:basic-user`. An overlay that adds a `persons` issuer adds its
own client, because no two clients share a `binding_issuer`:

| Client | `binding_issuer` | Declared in | Provisions |
|---|---|---|---|
| `basic-users` | `urn:ferroehr:basic-user` | `docker/ferroehr.dev.toml` | `ferroehr`, `ferroehr-admin`, `ferroehr-readonly` |
| `cnf-smart` | `urn:ferroehr:cnf-smart-login` | `docker/sut-smart.yml` | `cnf-smart-app`, the subject the conformance runner mints |
| `keycloak-dev` | `urn:ferroehr:keycloak-dev-login` | `docker/viewer/e2e-env.yml` | the dev realm's users, by their fixed ids |

Each lane loads the professional templates (`ferroehr definitions load`),
starts `serve`, then runs `scripts/scim-provision.sh` with its client and
users.

The conformance harness (`scripts/conformance.sh`) provisions the catalogue's
logins with `--name Veredictum`. They are the instrument's own logins, and
the catalogue states its committer as `{_type: PARTY_IDENTIFIED, name:
Veredictum}`. Giving the bound `PERSON` that name makes the stated committer
the caller's own, which the server accepts and replaces with the committer it
derives from the login. A committer naming anyone else is refused with `403`
(#5002, reported upstream in #5019).
