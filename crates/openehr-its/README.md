# openehr-its

**openEHR ITS (Implementation Technology Specifications) for Rust**: the
canonical JSON and canonical XML serialization of the openEHR RM, the
generated ITS-REST 1.1.0 API contract, and the OPT 1.4 and AOM2 archetype XML
codecs. The Simplified Formats (FLAT / STRUCTURED / Web Template),
RM-instance validation and the SMART on openEHR scope grammar live in the
separate [`openehr-sdt`](https://docs.rs/openehr-sdt) crate, which builds on
this one.

## What it provides

- **Canonical JSON**: `json::{to_canonical_json, from_canonical_json,
  from_canonical_value}`, with `_type` first, BMM field order, a STRICT reader
  (undeclared/duplicate keys refused), refusal paths naming the offending
  JSON node, and validation against the embedded official ITS-JSON RM schema.
  The `_type` → decode dispatch and declared-key table
  (`json_codec::generated::structural`) span **every** emitted generation of
  the spec crates at once; the XML, REST and OPT surfaces below are generated
  over their current generations (RM 1.2.0 / BASE 1.3.0 / AM 2.4.0).
- **Canonical XML**: generated `ToXml`/`FromXml` implementations over a
  `quick-xml` runtime, serving both published XSD lineages (the root
  namespace is a serialize-time choice).
- **ITS-REST contract**: generated DTOs, `#[async_trait]` server traits,
  per-group clients and route tables for every ITS-REST 1.1.0 API group. A
  server implements the traits over `axum`; a consumer calls a CDR through the
  clients.
- **Wire validation**: `wire_validate`, the wire-boundary dispatcher for the
  RM class invariants, refusing undeclared keys and routing every class to
  its invariant checks in `openehr_rm::validate`.
- **OPT 1.4** (`opt14`) and both AOM2 archetype XML codecs.

### Reading and writing canonical JSON

```rust
use openehr_its::json::{from_canonical_json, to_canonical_json};
use openehr_rm::prelude::Composition;

fn round_trip(text: &str) -> Result<String, openehr_its::json::JsonParseError> {
    let composition: Composition = from_canonical_json(text)?;
    Ok(to_canonical_json(&composition))
}
```

`from_canonical_json` is the strict reader: a body with an undeclared key, a
repeated member, a wrong `_type` or a malformed identifier is refused with the
JSON path of the offending node. `to_canonical_json` writes `_type` first and
the fields in BMM order, omitting absent optionals. With the
`schema-validation` feature, `json::validate_canonical` checks a
`serde_json::Value` against the embedded ITS-JSON RM schema.

## Features

`default = ["full"]` is every surface above. Underneath it the graph is
layered, so a consumer takes only what it reads:

| Feature | Adds | Pulls in |
|---|---|---|
| `json` | `json`, `json_codec`, `wire_validate` | serde, the five spec crates |
| `xml` | `xml`, `aom2`, `aom2_model` | `json` + `quick-xml` |
| `opt14` | `opt14` | `xml` |
| `schema-validation` | `json::validate_canonical` | `json` + `jsonschema`, the embedded RM schema |
| `rest` | `rest::generated` (DTOs, param structs, route tables), `rest::runtime::ApiError` | `json` + `http` |
| `rest-server` | the per-group `#[async_trait]` server traits, the `axum` response mapping of `ApiError` | `rest` + `axum`, `async-trait` |
| `rest-client` | the per-group clients `rest::generated::<group>::client`, the client runtime `rest::client` | `rest` + `reqwest`, `url`, `urlencoding`, `base64`, `backon`, `secrecy`, `async-trait` |

Every surface needs a feature: with `default-features = false` and nothing
else the crate compiles empty.

### On `wasm32-unknown-unknown`

```console
$ cargo add openehr-its --no-default-features --features opt14
```

That selection carries the openEHR content layers: canonical JSON, canonical
XML and OPT 1.4. `opt14` pulls `xml` and `json` in with it.
`schema-validation` and the three REST features stay out of it: `rest-server`
is the server half of the contract, which a client never implements, and
`rest-client` sends through `reqwest`. FerroEHR's CI builds the wasm selection
on every code change and clippies each REST feature on its own natively, so
the selections are checked rather than claimed.

## Calling a CDR

With `rest-client`, each ITS-REST API group has a generated client over one
configured `rest::client::Client`. Every operation takes its param struct
(path, query and header parameters: `Prefer`, `If-Match`, and on the commit
operations the committal-metadata headers `openehr-version`,
`openehr-audit-details` and `openehr-template-id` the ITS-REST overview
defines) and answers an outcome enum with one variant per status the OpenAPI
documents for it; declared response headers such as `ETag` and `Location`
arrive in a per-answer `headers` struct, and every `4xx` variant carries the
answer body as an `ErrorBody` (the bytes as received, decoded as the ITS-REST
`Error` when they are one, with `message()` and `validation_errors()`). A
path parameter is percent-encoded with `:` kept literal, so a version uid
travels as the specification writes it. The base is a `url::Url` ending in
the API version segment, so the caller depends on the `url` crate too; a
credential secret is a `secrecy::SecretString`, built from a `String` with
`into()`.

```rust,no_run
use std::time::Duration;

use openehr_its::rest::client::{Client, Credentials, ReqwestTransport};
use openehr_its::rest::generated::ehr::EhrGetByIdParams;
use openehr_its::rest::generated::ehr::client::{EhrClient, EhrGetByIdOutcome};

async fn fetch_ehr(ehr_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let transport = ReqwestTransport::with_timeout(Duration::from_secs(30))?;
    let base = url::Url::parse("https://cdr.example.org/openehr/v1")?;
    let client =
        Client::new(transport, base)?.with_credentials(Credentials::basic("reader", "secret"));

    let params = EhrGetByIdParams { ehr_id: ehr_id.to_owned(), accept: None };
    match EhrClient::new(&client).ehr_get_by_id(&params).await? {
        EhrGetByIdOutcome::Ok { body, headers } => {
            // `body` is the decoded `openehr_rm` EHR.
            let _ehr = body;
            let _content_type = headers.content_type;
        }
        EhrGetByIdOutcome::NotFound { body } => {
            // The service's own diagnostics, when it sent any.
            let _message = body.message();
        }
    }
    Ok(())
}
```

A status the operation does not document is `ClientError::UndocumentedStatus`;
`401`, `403` and a `5xx` are `ClientError::Unauthorized`, `Forbidden` and
`ServiceFailure`, each carrying the answer body as an `ErrorBody`. Idempotent requests (`GET`, `HEAD`, `OPTIONS`, `PUT`,
`DELETE`) are retried after a transport failure, a timeout or a `5xx`, within
the budget `Client::with_retry` sets; a `POST` is sent once. The HTTP engine is
the `Transport` trait, so another `http`-speaking client can replace
`ReqwestTransport`, and `Client::execute` sends a request the typed surface
does not build.

## Generated code — do not edit

Most of this crate is generated, and every generated file carries a
`// @generated` header. The generated halves are emitted
deterministically by [`openehr-codegen`](https://github.com/FerroHEALTH/FerroEHR/tree/main/tools/openehr-codegen)
from the vendored openEHR artifacts: the XML codec from the ITS-XML XSDs plus
the BMM field model, the REST contract from the ITS-REST OpenAPI documents,
the `_type` dispatch table from the BMM, and the three archetype XML codecs
(`opt14`, `aom2`, `aom2_model`) from their XSD closures. Changes to those
belong in the emitter, never in the generated output. The hand-written parts
are the ones the generated code cannot ship without: the `quick-xml` and REST
runtimes, the canonical-JSON entry points and the wire-validation dispatcher.
They are edited normally.

## Versioning

The package version is the crate's **own independent SemVer line**: it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification. The implemented
spec version is always available at runtime as the crate-level constant
`openehr_its::SPEC_VERSION` (`"1.1.0"`, the ITS-REST release this crate
implements, with the ITS-XML and ITS-JSON artifacts pinned alongside),
independent of the package version.

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## Attribution — the embedded openEHR artifact

Exactly one third-party file travels inside this package:

- **Packaged path:** `schemas/json/openehr_rm_1.1.0_all.json`
- **Upstream:** [`openEHR/specifications-ITS-JSON`](https://github.com/openEHR/specifications-ITS-JSON),
  path `components/openehr_rm_1.1.0_all.json`, commit
  `5acae056248e917a4b4c56f7e712f4fcfeb616a6` (`master` — ITS-JSON is
  DEVELOPMENT status and has no numbered release)
- **Copyright:** openEHR Foundation; redistributed **verbatim** under
  Apache-2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0),
  <https://github.com/openEHR/specifications-ITS-JSON/blob/master/LICENSE>)
- **Role here:** the consolidated ITS-JSON RM 1.1.0 JSON Schema, embedded as
  `openehr_its::json::RM_SCHEMA_JSON` under the `schema-validation` feature and
  used as the validation oracle for canonical-JSON output — it is not a code
  source.

If you redistribute this crate, that file and this attribution travel with it.
The rest of the vendored ITS-JSON tree, and the ITS-XML XSDs and ITS-REST
OpenAPI documents the generated code was emitted from, stay in the repository
and are not packaged.

## License

Apache License 2.0 ([`LICENSE-APACHE-2.0`](LICENSE-APACHE-2.0), SPDX
`Apache-2.0`), for the Rust code and the embedded schema alike. The generated
codecs and contract derive from the official openEHR machine-readable
specification artifacts (the ITS XSD, OpenAPI and BMM files), which openEHR
publishes under Apache-2.0, and the package embeds the official ITS-JSON RM
schema attributed above. Every file names Cadasto B.V. and the openEHR
Foundation as copyright holders.

Version history: up to 0.0.59 the crate was published under Apache-2.0;
0.0.60 to 0.0.67 were published as `BUSL-1.1 AND Apache-2.0` and keep that;
0.0.68 was never published; from 0.0.69 it is Apache-2.0 again. The hand-written Simplified Formats,
RM-instance validation and SMART scope grammar that carried the BUSL-1.1
position moved to [`openehr-sdt`](https://docs.rs/openehr-sdt).

## Part of FerroEHR

This crate is the serialization and REST-contract layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
