# openehr-sdt

**The openEHR Simplified Formats for Rust**: FLAT and STRUCTURED composition
reading and writing, Web Template building, TDD import, template-independent
RM-instance validation, and the SMART on openEHR scope grammar. Hand-written
over the [`openehr-its`](https://docs.rs/openehr-its) wire layer (Apache-2.0).

The name comes from the Simplified Data Template specification
(`docs/specs/openehr/ITS-REST/docs/simplified_data_template/`), the ITS-REST
document family these formats grew out of. As of ITS-REST Release 1.1.0 that
document is marked RETIRED in favour of the Simplified Formats specification,
and Simplified Formats is what this crate implements.

## What it provides

- **`openehr_sdt::flat`**: the Simplified Formats. FLAT and STRUCTURED data
  instances, the Web Template model and builder, and the TDD to COMPOSITION
  converter. The authority is the STABLE ITS-REST 1.1.0 Simplified Formats
  specification (`docs/specs/openehr/ITS-REST/docs/simplified_formats/`):
  `master04` for field identifiers, the node-id algorithm and the
  FLAT/STRUCTURED algorithms, `master05` for the per-RM-type mapping tables,
  `master06` for the `ctx/` vocabulary. The entry points are in
  `flat::convert`: `composition_from_flat`, `composition_to_flat`,
  `composition_from_structured`, `composition_to_structured`,
  `flat_to_structured` and `structured_to_flat`, each converting against a
  `flat::webtemplate::model::WebTemplate`. The `submitted_composition_from_flat`
  and `submitted_composition_from_structured` variants add the master04
  §Validation input-side checks a submitted body runs.
- **`openehr_sdt::rm_instance`**: template-independent validation of a whole
  RM instance tree. `validate_rm_and_terminology` runs the RM-invariant and
  terminology passes, `validate_composition` composes the template-driven pass
  of `flat::validation` over them, and both report `ValidationMessage` values.
- **`openehr_sdt::smart_scopes`**: the SMART on openEHR resource-scope grammar
  (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master08-scopes.adoc`),
  plus the launch-context scopes of master07 §Context Selection and master09
  §Experimental: Episode Context. The parser is total: a scope string the
  grammar does not recognise becomes `SmartScope::Other` and is kept inert.

The SM SIM-B and SDF model documents are DEVELOPMENT-state and are not
implemented; their terse string encodings are not accepted. No vendor
implementation is used as an oracle.

### Parsing a scope claim

```rust
use openehr_sdt::smart_scopes::SmartScope;

let scopes = SmartScope::parse_all("openid patient/composition-*.cru");
for scope in &scopes {
    if let SmartScope::Resource(resource) = scope {
        assert!(resource.permissions.read);
        assert!(!resource.permissions.delete);
    }
}
```

## Features

| Feature | Adds | Pulls in |
|---|---|---|
| (none) | `smart_scopes` | nothing: std only |
| `flat` | `flat`, `rm_instance` | `openehr-its` with `opt14`, the five generated spec crates, serde, `serde_json`, `thiserror`, `quick-xml`, `indexmap`, `regex`, `fancy-regex` |
| `cache` | `flat::cache` (the async Web Template cache) | `flat` + `moka` |
| `full` (default) | everything above | `flat` + `cache` |

With `default-features = false` and nothing else the crate compiles to
`smart_scopes` alone, with no dependency of any kind. A browser client can
then parse SMART scope strings with the same grammar a server enforces.

### On `wasm32-unknown-unknown`

```console
$ cargo add openehr-sdt --no-default-features --features flat
```

That selection carries the Simplified Formats and the RM-instance validation;
`cache` stays out. For the scope grammar alone, drop `--features flat`.
FerroEHR's CI checks both selections for that target on every code change.

## Hand-written, not generated

Everything in this crate is hand-written. The specifications it implements are
prose sub-specifications of ITS-REST with no machine-readable model to generate
from. It consumes the generated `openehr-rm` and `openehr-am` types and the
canonical JSON entry points of `openehr-its` directly, and never re-models the
RM.

## Versioning

The package version is the crate's **own independent SemVer line**. It tracks
this implementation's code and moves freely with fixes and improvements, never
with the vendored openEHR specification. The implemented spec version is
available at runtime as the crate-level constant `openehr_sdt::SPEC_VERSION`
(`"1.1.0"`, the ITS-REST release that Simplified Formats and SMART App Launch
belong to), independent of the package version.

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## License

Business Source License 1.1 ([`LICENSE`](LICENSE), SPDX `BUSL-1.1`), the
licence of the FerroEHR application: all non-production use is free, production
use is free for Non-Commercial Purposes, and any other production use, hosting
for third parties or distribution for a fee needs a commercial licence from the
Licensor, Cadasto B.V. Each version becomes Apache License 2.0 four
years after it is published. The crate is hand-written; the openEHR
specification text it implements is the authority but is not embedded in the
package. The crate was first published at 0.0.68, built against the 0.0.67 siblings;
from 0.0.69 it moves in lockstep with them. Its code was published as
part of `openehr-its` before that.

## Part of FerroEHR

This crate is the Simplified Formats layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
