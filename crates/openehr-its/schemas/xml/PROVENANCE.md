# Vendored ITS-XML schemas (provenance + serving policy)

Reproduced by: `scripts/vendor/codegen-inputs.sh` (`--check` verifies the committed files against the pin).

XSD schemas from `openEHR/specifications-ITS-XML`, vendored **verbatim** (full
upstream `components/` trees) for canonical-XML (de)serialization. Reference /
validation inputs and the `emit-xml` codegen oracle; no code here.

Source repo: https://github.com/openEHR/specifications-ITS-XML
License: Apache-2.0 (the upstream repo's `LICENSE`; root reference copy
`LICENSE-APACHE-2.0`)

## Two lineages are vendored — they differ by NAMESPACE **and by RM generation**

The ITS-XML repo's `Release-2.0.0` restructure changed the XML target namespace
from `http://schemas.openehr.org/v1` to `http://schemas.openehr.org/v2`
repo-wide and re-stamped every historical release folder to `v2`. So the
*namespace* is an axis independent of the RM model version. Each lineage is
vendored as the upstream `components/` tree, verbatim:

| Vendored dir | Repo ref | Namespace | What it is |
|---|---|---|---|
| `its-xml-1.0.2-nsv1/` | tag `Release-1.0.2v2` @ `f7a937778bf9ea43b01b0f9d8a616e47f35017c1` | `http://schemas.openehr.org/v1` | The STABLE pre-2.0.0 bundle: flat `ALL/` (11 XSDs) + `AOM2/` (6 XSDs + examples). |
| `its-xml-2.0.0-nsv2/` | tag `Release-2.0.0v2` @ `de8b37ba6c9a5e126623a063cafba3b58ebf1107` (also repo HEAD at vendoring) | `http://schemas.openehr.org/v2` | The 2.0.0 release (TRIAL upstream): full `components/` — RM (1.0.2/1.0.3/1.0.4/1.1.0/latest), BASE (1.1.0/1.2.0/latest), AM (1.4/latest), OET (1.0.1/latest), QUERY/latest (70 XSDs). |

Fetched: 2026-07-04. (The `openEHR/v1/Template` namespace in the OET/Template
schemas is the separate template-document namespace, not the RM namespace.)

**The two bundles are NOT the same schema in two namespaces.** The 2.0.0
restructure also re-packaged the schemas per component and per RM release, and
the flat `Release-1.0.2v2` bundle was never re-issued against a newer RM — so
it is frozen at an RM generation older than the RM 1.2.0 model this repository
generates from. Concretely, measured mechanically by the gate
`crates/openehr-its/tests/it/xml_xsd_validity.rs`:

- The nsv1 bundle publishes only `components/ALL/` (11 XSDs) + `components/AOM2/`
  — no `Ehr.xsd`, no `Demographic.xsd`. **50 concrete RM 1.2.0 classes have no
  `xs:complexType` there at all** (EHR, EHR_STATUS, CONTRIBUTION, the
  `VERSIONED_*` containers, the demographic PARTY types, `DV_SCALE`,
  `ITEM_TAG`, …).
- Where nsv1 *does* declare the class it declares **23 fewer attributes across
  17 classes** — `FOLDER.details`, `ELEMENT.null_reason`,
  `CODE_PHRASE.preferred_term`, `DV_QUANTITY.units_system`/`units_display_name`,
  `FEEDER_AUDIT_DETAILS.other_details` (nsv2 `RM/Release-1.1.0`);
  `ISM_TRANSITION.reason` and the EHR-Extract members (nsv2
  `RM/Release-1.0.3`); `ENTRY.workflow_id`, which every nsv2 RM release folder
  declares and nsv1 never does.
- The nsv2 lineage **cannot be compiled by a conformant XSD processor**: its
  `archetypeNodeId` `pattern` facet uses Perl `(?:…)` groups, which XML Schema
  Part 2 Appendix F (<https://www.w3.org/TR/xmlschema-2/#regexs>) does not
  define.

The full per-attribute adjudication is pinned in that gate and in
Veredictum's `artifacts/registers/ambiguities.yaml` AMB-185. A served v1
document may therefore carry RM 1.2.0 members the v1 XSD predates; the codec
does not trim the RM model to fit an older schema packaging.

## Which namespace does the CDR serve? — v1 default, v2 negotiated

- Upstream marks 2.0.0 **TRIAL** and directs stable consumers to
  `Release-1.0.2` (`docs/specs/openehr/ITS-XML/README.adoc` §Releases and IM
  Versions), so the **v1** namespace is the released-STABLE lineage and the
  served default under the released-spec policy (`docs/VERSIONS.md`).
- Owner ruling 2026-07-28 (#196): the **v2** namespace is served **on
  request** via the `version` media-type parameter on the canonical-XML
  media type (`Accept: application/xml; version=2`). No openEHR spec governs
  namespace selection on the REST wire — the parameter is our own
  design/extension (register AMB-169, `option_select`).

## Generation status — SETTLED (one codec, both namespaces)

- RM *model* stays 1.2.0 internally (JSON serialization unaffected).
- `emit-xml` generates ONE impl set serving **both** wire lineages — our two
  SERIALIZED documents differ only by the root `xmlns`, selected at serialize
  time (`crates/openehr-its/src/xml/runtime.rs`); this is NOT an AM-style dual
  generation. (The one codec always writes the full RM 1.2.0 model; what the
  two *schemas* accept differs, per the section above.)
