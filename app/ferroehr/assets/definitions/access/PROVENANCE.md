# Access definition: provenance

One ADL 2 archetype, the default definition of an `EHR_ACCESS` in the
`access_directives` scheme. The binary carries it
(`ferroehr::service::ehr::access_settings::DEFINITIONS`), and each boot
stores it when no definition of its id is stored, never over a stored one.
It is an example: FerroEHR reads `EHR_ACCESS.settings` by the codes of its
access control terminology, never by this archetype's id, and a deployment
versions or replaces it through the definition API.

1. `openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls`: the `EHR_ACCESS`
   root and its `settings`, constrained to the RM's
   `ACCESS_CONTROL_SETTINGS`.

## Source

FerroEHR's own work. openEHR CKM publishes no `EHR_ACCESS` archetype, and the
RM leaves `ACCESS_CONTROL_SETTINGS` "Currently implementation dependent" (RM
EHR IM, `org.openehr.rm.ehr.access_control_settings.adoc`).

## Our own modelling

- **The settings are constrained to the abstract RM class.** Their concrete
  class, `ACCESS_DIRECTIVE_SETTINGS`, is in FerroEHR's draft access control
  package, which the ADL 2 validator does not read: it checks every object
  type against RM 1.2.0, so a constraint naming a draft class is refused.
  The write check of every `EHR_ACCESS` holds the settings to the draft class
  and its invariants instead.
- The node ids are ADL 2 id-codes and the terminology is English only.

## Licence

`BUSL-1.1`, Cadasto B.V., recorded in `REUSE.toml`.
