---
name: bmm-schema-validity-landmarks
description: Where the P_BMM/BMM schema-validity rules live (LANG bmm_persistence master04-syntax + bmm3 master05/06/08) and the confirmed closure/naming defect landmarks in the vendored BMM files
metadata:
  type: reference
---

# BMM schema validity + closure — where the rules and the defects live

## The rule text (LANG component)
- `LANG/docs/bmm_persistence/master04-syntax.adoc` — the P_BMM serialisation
  rules. Section anchors (verified 2026-08-21): §Header Items L66;
  **§Inclusions L95** (an ODIN example ONLY — `id = <"openehr_basic_types_1.0.2">`,
  zero prose, so "inclusion is the only widening mechanism" is an INFERENCE);
  §Package Definition L106 with the two NOTEs — **L112 "only classes defined in
  the same schema can be referenced in the package section in that schema"**;
  §Classes for Primitive Types L128 ("just normal class definitions within a
  `primitive_types` block") + its NOTE L133 (container types are explicit;
  contains the upstream typo "exlicit"); §Container Properties L209;
  §Generic Classes L327 (L331 "necessitates the use of the BMM meta-type
  `P_BMM_SINGLE_PROPERTY_OPEN`"); container `type` vs `type_def` rule **L453**
  ("use 'type' for simple string type refs; use `_type_def_` for structure
  types"); **schema_id = `<rm_publisher>_<schema_name>_<rm_release>` L79**.
- `LANG/docs/bmm3/master05-core-model.adoc` — §Naming Convention L11-13
  (case-insensitive: `"Hashable"` = `"HASHABLE"`, underscores significant);
  §Model Semantics → **§Packages L27-31**: L29 "every class is contained within
  exactly one package", L31 "not used as namespaces … all classes in a BMM model
  should be uniquely named".
- `LANG/docs/bmm3/master06-core-types.adoc` — §Type Conformance L230, per-form
  §Conformance subsections (Simple L53: "A conforms to B iff for `base_class` of
  A, `all_ancestors()` contains B").
- `LANG/docs/bmm3/master08-core-features.adoc` §Differential and Flat Form L49.
- `LANG/docs/bmm3/master13-model_semantics.adoc` L75-76 = literal `[.tbd] TBD`
  (redefinition semantics unpublished).
- Type-form multiplicities: `LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_{simple,generic,container}_type.adoc`
  — base_class/container_type all `1..1`; `bmm_open_type.adoc` §Description
  "The parameter must be in the type declaration of the owning BMM_CLASS".
- `…bmm_persistence.p_bmm_schema.adoc` — `merge` `__Pre_other_valid__:
  includes_to_process.has (included_schema.schema_id)`; `…bmm.bmm_include_spec.adoc`
  keys on `id: String` alone. `…bmm.bmm_schema_core.adoc` L69 = schema_id is
  "Derived name … based on model publisher, model name, model release".

## Where the BMM files are (json + odin; yaml no longer vendored since 2026-10-14)
`tools/openehr-codegen/vendor/bmm/components/<COMP>/{json,odin}/…` — json is
the codegen input; the BASE 1.3.0 / RM 1.2.0 / TERM 3.1.0 odin files are stale
ITS-BMM copies that lack the json overlay changes (see that dir's PROVENANCE.md). A second copy of the CURRENT pins only lives under
`docs/specs/openehr/<COMP>/computable/BMM/` (RM has ONLY 1.2.0 there; RM 1.1.0
exists only in the codegen vendor tree).

## Confirmed defect landmarks (re-verified first-hand 2026-08-21)
- **No `includes` block at all**: term 3.0.0/3.1.0, rm 1.0.2/1.0.3, lang 1.0.0,
  base 1.0.4. **base 1.0.4 is the exception that matters**: it carries its own
  32-entry `primitive_types` block (String/List/Boolean/Integer all present), so
  only `BMM_OPEN_TYPE` is unresolvable there — the "every primitive reference is
  unresolvable" generalisation is false for that one schema.
- am 2.2.0 includes base 1.1.0 only → `ARCHETYPE.rules: List<STATEMENT>` dangles;
  am 2.3.0 includes base 1.2.0 + **lang 1.0.0** → `List<STATEMENT_SET>` dangles
  (STATEMENT/STATEMENT_SET are published in lang 1.1.0 `org.openehr.lang.beom.core`).
- **The two LANG 1.1.0 schemas render byte-identical headers** (rm_publisher
  openehr / schema_name lang / rm_release 1.1.0 / schema_revision 1.1.0.2) →
  one schema_id `openehr_lang_1.1.0`. am 2.4.0 includes that id and needs both:
  `STATEMENT_SET`/`ASSERTION`/`EXPR_LEAF` are v2.x-only, `EL_BOOLEAN_EXPRESSION`
  is v3-only (`P_ARCHETYPE.rules` is `List<EL_BOOLEAN_EXPRESSION>`).
- `BMM_ENUMERATION.item_values: List<T>` with no `generic_parameter_defs` on the
  class or on BMM_CLASS (v2.x json/odin alike); bmm3 fixed it to
  `List<BMM_PRIMITIVE_VALUE>`. Beware: the same schema ALSO has
  `P_BMM_ENUMERATION.item_values: List<Any>` — two `item_values` sites per file.
- `EL_CASE<T>.value_constraint: C_OBJECT` in the bmm3 schema (includes BASE only;
  AM includes LANG, so the fix cannot be an include). Its class page's doc string
  is truncated to "Constraint on".
- **P_ARCHETYPE_SLOT.excludes AND .includes** both state `container_type: List`
  with NO target type (json/odin). The complete `List<ASSERTION>` pair lives
  on `ARCHETYPE_SLOT` — whose class page is
  `AM/docs/UML/classes/org.openehr.am.aom2.archetype_slot.adoc` L23, included by
  `AM/docs/AOM2/master04.5-constraint_model-class_definitions.adoc` L429.
  **There is no `AM/docs/AOM2/master07.05`** (that number exists only as
  `AM/docs/ADL2/master07.05-adl_identification.adoc`).
- Duplicate containment across the RM→BASE include: AUTHORED_RESOURCE /
  RESOURCE_DESCRIPTION / TRANSLATION_DETAILS / RESOURCE_DESCRIPTION_ITEM
  (`org.openehr.rm.common.resource` vs `org.openehr.base.resource`) and
  **CODE_PHRASE** (`org.openehr.rm.data_types.text` vs
  **`org.openehr.base.base_types.terminology`** — NOT foundation_types, even
  though BASE `docs/foundation_types/master00-amendment_record.adoc` SPECAM-82
  says "Add legacy CODE_PHRASE class to Foundation Types"; the docs home is
  `BASE/docs/base_types/master06-terminology_package.adoc`, and
  `base.foundation_types.terminology` holds only Terminology_code/_term).
- Case-folded collision: AM 1.4.0 `CARDINALITY` (class_definitions,
  `interval: Interval<Integer>` + is_bag/is_list/is_set) vs BASE 1.3.0
  `Cardinality` (**primitive_types** block, `interval: Multiplicity_interval`,
  no functions). AM 1.4.0 also redefines `ARCHETYPE.uid: HIER_OBJECT_ID` over
  `AUTHORED_RESOURCE.uid: UUID` (HIER_OBJECT_ID ancestors = UID_BASED_ID,
  OBJECT_ID — no UUID, so non-conformant).
- **AM 1.4.0 BMM `includes` = openehr_base_1.3.0** while
  `AM/docs/AOM1.4/masterAppC-rm_dependencies.adoc` §"RM 1.0.2 common.resource
  package" names the RM package, and RM `docs/common/master08-resource_package.adoc`
  reserves its retained copy for AOM 1.4 — three-way contradiction.

Related: [[adl2-parser-spec-location]], [[lexical-layer-per-language-location]],
[[el-grammar-landmarks]].
