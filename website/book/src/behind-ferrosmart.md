<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Behind FerroSMART

FerroSMART is the FerroHEALTH family's SMART App Launch 2.x authorization
server. It signs people in at any OpenID Connect provider and uses that
provider for login alone. It issues plain RFC 9068 access tokens that
carry the launch context: `patient`, `encounter`, `fhirUser` and, at an
openEHR base, `ehrId`. FerroEHR runs behind it as a resource server. The
person never signs in to FerroEHR, and FerroEHR never sees the provider's
tokens. FerroSMART's architecture describes the whole deployment in
§13.21, A product behind FerroSMART, of its [architecture](https://github.com/FerroHEALTH/FerroSMART/blob/ferrosmart-v0.2.0/docs/architecture.md).
This page is FerroEHR's side of it.

```text
 SMART app ── /authorize, /token ──▶ FerroSMART ── login ──▶ OpenID Connect provider
     │                                   │
     │  Bearer access token              │ jwks_uri
     ▼                                   ▼
 FerroEHR (resource server) ── reads the token under the `ferrosmart` preset
```

## FerroSMART serves the openEHR base

FerroSMART issues a token only for a base it serves. Its
`[[ferrosmart.resource]]` table names FerroEHR's openEHR REST base as an
`openehr` resource. Its `subject_namespace` is the namespace FerroSMART's
patient picker looks EHRs up under:

```toml,ferrosmart
[[ferrosmart.resource]]
base = "https://ehr.example.org/ferroehr/rest/openehr/v1"
kind = "openehr"
subject_namespace = "examples"
```

## FerroEHR trusts FerroSMART

One `[[auth.issuer]]` table names FerroSMART's issuer with `kind =
"persons"` and `profile = "ferrosmart"`
([Authentication & access](installation/config-auth.md)). The preset keeps
every `rfc9068` default but two: `typ` is `at+jwt` alone (RFC 9068 §2.1),
and every token carries `scope` (SMART App Launch 2.2.0, Obtain access
token, Response). A token without `scope` is refused with `401`.

```toml
[[auth.issuer]]
issuer = "https://smart.example.org"
kind = "persons"
profile = "ferrosmart"
audience = ["https://ehr.example.org/ferroehr/rest/openehr/v1"]
```

- **A person through a SMART app.** FerroEHR reads who may act for a person
  from the CDR, never from the configuration. Grant each SMART app
  `act_for_persons` on the `AGENT` carrying its `client_id` under
  FerroSMART's issuer ([Enterprise identity providers](identity-providers.md#who-the-caller-is)).
  A person's token through a granted app reads as that person, delegated to
  the app, and the access record carries `acting_mode = delegated` and the
  app. A token through an app the CDR grants nothing is no caller and is
  refused with `401`.
- **FerroSMART's patient picker.** The picker looks a patient's EHR up as
  the person who signed in, with a token whose `client_id` is FerroSMART's
  own issuer (FerroSMART `docs/architecture.md` §13.6 item 4). Grant that
  client `sign_in_persons` on an `AGENT` carrying FerroSMART's issuer as its
  `client_id`, so the lookup reads as the person themselves. Without the
  grant, FerroEHR refuses the lookup with `401`, and a standalone launch
  with `launch/patient` stops at the picker.
- **A Backend Services client.** A client credentials token's `sub` is its
  `client_id` (RFC 9068 §2.2), so the same table reads it as the system: a
  `persons` issuer also signs its machines' app-only tokens. Its grants in
  the CDR decide what it reaches, as for any system caller.
- **One table per issuer.** Name FerroSMART in this one table. A second
  `[[auth.issuer]]` table of the same issuer, such as a `machines` table
  beside it, refuses the start, naming the issuer
  (FerroHEALTH/FerroSMART#591).

The scopes a token grants still meet FerroEHR's own access decisions on top.

## A person's own record

FerroEHR reads who a record belongs to from the party binding and never from
the token alone. A person whose login binds to the `PERSON` that an EHR's
`EHR_STATUS.subject` names is that EHR's subject. On that EHR they read their
record and write its `EHR_ACCESS` settings as its subject (EHDS Art. 3), with
no role and no purpose of use. The access log records each access as their own.

The binding is the source because an openEHR launch token names its record
by `ehrId` and carries no `fhirUser` and no `patient` claim.

FerroSMART's patient picker looks the EHR up with a token that carries no
role:

- A person holding no role reaches only the EHRs whose subject is their
  `PERSON`.
- A patient signs in at the same `persons` issuer as staff. Their `PERSON`
  holds no professional `ROLE`, so FerroEHR binds the login as that person:
  they hold no role, whatever the token asserts, and reach only the EHRs
  whose subject is their `PERSON`. Their commits name `PARTY_SELF`.
- Their lookup of their own subject finds their EHR. A lookup of anyone
  else's subject answers `404`, as a subject no EHR names does.
- A professional's lookup through the picker needs their roles on the
  picker's token (FerroHEALTH/FerroSMART#716).

## The discovery document describes FerroSMART

With `[auth.smart] discovery = true`,
`GET https://ehr.example.org/ferroehr/rest/openehr/v1/.well-known/smart-configuration`
describes the issuer `ferrohealth-smart`'s shared rule picks
([SMART App Launch](smart-app-launch.md)). With FerroSMART as the one
`persons` issuer, that is FerroSMART: the document names its
`authorization_endpoint` and `token_endpoint`, so an app that starts at
FerroEHR's base finds FerroSMART (SMART App Launch 2.2.0, Conformance,
Metadata). When FerroEHR trusts another issuer that signs user tokens
beside it, `[auth] described_issuer` names FerroSMART.

## Try it

The quickstart runs FerroEHR behind FerroSMART with Keycloak or ZITADEL
upstream ([Docker Compose](installation/compose.md#behind-ferrosmart)), and
[Enterprise identity providers](identity-providers.md#behind-ferrosmart) lists
every upstream FerroSMART supports with its setup.

No specification governs this deployment shape: our own design, built on
SMART App Launch 2.2.0 and RFC 9068.
