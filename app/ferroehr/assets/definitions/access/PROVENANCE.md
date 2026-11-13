# Access definition: provenance

Two versions of one ADL 2 archetype, the default definition of an
`EHR_ACCESS` in the `access_directives` scheme. The binary carries both
(`ferroehr::service::ehr::access_settings::DEFINITIONS`), and each boot
stores each one when no definition of its id is stored, never over a stored
one.
It is an example: FerroEHR reads `EHR_ACCESS.settings` by the codes of its
access control terminology, never by this archetype's id, and a deployment
versions or replaces it through the definition API.

1. `openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls`: the `EHR_ACCESS`
   root and its `settings`, constrained to the RM's
   `ACCESS_CONTROL_SETTINGS`. Kept as published.
2. `openEHR-EHR-EHR_ACCESS.access_directives.v1.1.0.adls`: the `settings`
   constrained to `ACCESS_DIRECTIVE_SETTINGS` and its attributes to their
   classes. Every settings value it admits is one `v1.0.0` admits, so it is a
   minor version (AOM2 `master03-archetype_package.adoc`: a later minor
   version cannot create data that is not valid with respect to the prior
   version).

## Source

FerroEHR's own work. openEHR CKM publishes no `EHR_ACCESS` archetype, and the
RM leaves `ACCESS_CONTROL_SETTINGS` "Currently implementation dependent" (RM
EHR IM, `org.openehr.rm.ehr.access_control_settings.adoc`).

## Our own modelling

- **The settings name a draft class.** `ACCESS_DIRECTIVE_SETTINGS` is in
  FerroEHR's draft access control package, which is no openEHR release. The
  definition path validates every ADL 2 archetype against RM 1.2.0 extended
  by the class tables of the draft packages (`ferroehr::draft_model`), where
  the class conforms to the RM's `ACCESS_CONTROL_SETTINGS` (AOM2
  `master11-rm_adaptation.adoc`). The validator over RM 1.2.0 alone refuses
  `v1.1.0` (`VCORM`). The write check of every `EHR_ACCESS` still holds the
  settings to the class, its invariants and its terminology groups.
- The node ids are ADL 2 id-codes and the terminology is English only.

## Licence

`BUSL-1.1`, Cadasto B.V., recorded in `REUSE.toml`.
