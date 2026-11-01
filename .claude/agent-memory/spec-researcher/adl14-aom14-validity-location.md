---
name: adl14-aom14-validity-location
description: Where the ADL 1.4 / AOM 1.4 validity rules, the I_DEFINITION_ADL14 SM interface, and the adl1.4 REST operations live (no ADL1.4→ADL2 conversion endpoint exists)
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

# ADL 1.4 / AOM 1.4 validity + ADL14 REST/SM location

## AOM 1.4 invariants (the catalogue is CLASS-TABLE-based, not V-coded)
AOM1.4 does NOT use `*Vxxx*:` codes like AOM2. Its invariants live in the
per-class `*Invariants*` rows of the UML export tables:
`docs/specs/openehr/AM/docs/UML/classes/org.openehr.am.aom14.<class>.adoc`,
included by `AM/docs/AOM1.4/master0{3,4,7}-*.adoc`. Key files: archetype
(Inv_concept_valid, Inv_specialisation_validity, Inv_invariants_valid,
Inv_uid_validity, Inv_version_validity, Inv_description_valid,
Inv_original_language_valid + the is_valid/node_ids_valid/
internal_references_valid/constraint_references_valid FUNCTIONS),
c_attribute (Rm_attribute_name_valid, Existence_set, Children_validity),
c_complex_object (Attributes_valid), c_defined_object (Assumed_value_valid),
c_primitive_object (Item_valid), archetype_slot (Includes/Excludes/Validity),
archetype_internal_ref (Consistency, Target_path_valid), constraint_ref
(Consistency), c_single_attribute (Members_valid), c_date/c_time/c_date_time
(Pattern_validity + validity_kind cascade), assertion (Tag_valid,
Expression_valid BOOLEAN), archetype_ontology (concept_code_validity,
Term_bindings_validity, Parent_archetype_valid, Original_language_validity),
archetype_term (Code_valid). Prose rules: `master04 §Node_id and Paths`
(sibling-node unique node_id; node_id links definition↔ontology),
`master07 §Specialisation Depth` (at-code '.' specialisation coding;
depth 0 = standalone) + `master07 §Term_definitions` ("text"+"description"
mandatory per term). c_boolean/c_string/c_integer/c_real/c_duration/
c_ordinal/ordinal/c_quantity/c_quantity_item/c_coded_text have NO invariants.

## ADL 1.4 document's own validity rules (THESE are V-coded)
`AM/docs/ADL1.4/master08-adl.adoc §Validity Rules` (L535+): VARID, VARCN,
VARDF, VARON, VARDT (Global Archetype Validity); VATDF, VACDF (Coded Term
Validity); VDFAI, VDFPT (Definition Section). Section structure grammar +
`archetype/specialise/concept/language/description/definition/invariant/
ontology` order at L588 (`=== Grammar`); at-code/ac-code rules at
`§Node Identification` L42 + `§Local Constraint Codes` L46.

## I_DEFINITION_ADL14 (SM)
`SM/docs/UML/classes/i_definition_adl14.adoc` (included by
`SM/docs/openehr_platform/master04-definition_package.adoc` L55). Ops:
has_archetype, valid_archetype, upload_archetype (Post has_archetype; err
invalid_archetype; "must be valid to succeed"), get_archetype (err
artefact_does_not_exist), list_archetypes, list_matching_archetypes,
delete_archetype, has_opt, valid_opt, upload_opt (Pre valid_opt(an_opt); err
invalid_template), get_opt, list_opts, list_matching_opts, delete_opt,
archetypes_count, opts_count. NOTE spec quirk: params typed as AOM2 ARCHETYPE
even for ADL1.4. NO convert/migrate/upgrade op in SM.

## adl1.4 REST operations (ITS-REST)
`docs/specs/openehr/ITS-REST/specifications/operations/definition_template_
adl1.4_{list,upload,get,example_get}.yaml`; paths in `definition.openapi.yaml`
L25-35: POST+GET /definition/template/adl1.4, GET .../{template_id}, GET
.../{template_id}/example. ONLY templates/OPTs — NO adl1.4 ARCHETYPE REST op
(archetypes are SM-only). Upload: Content-Type application/xml only, Prefer
(rep/minimal/identifier default minimal), 201/400/409. Get/example: Accept
application/{json,xml,openehr.wt+json}(+wt.flat/structured for example),
200/400/404/406. Bundled OAS: `crates/openehr-its/vendor/rest-oas/
definition-{codegen,html,validation}.openapi.yaml`.
**CRUCIAL: NO ADL1.4→ADL2 conversion/migration/upgrade endpoint exists
anywhere** (grep convert/migrate/upgrade across ITS-REST ops + vendored OAS +
SM definition = none). archie's ADL14ConversionUtil is prior-art only; the
spec defines no conversion API.
