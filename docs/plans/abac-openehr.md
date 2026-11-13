# ABAC as openEHR content: model, BMM, mappings and the place of Cedar (tracker issue #5487)

- Status: in-progress (the design, for the owner's review; nothing is built
  and nothing is proposed outside this repository until the owner decides)
- Started: 2026-11-12
- Consumes: openEHR RM 1.2.0 (EHR IM `EHR_ACCESS`, `ACCESS_CONTROL_SETTINGS`,
  the Entry package; Demographic IM `PARTY`, `ROLE`, `CAPABILITY`,
  `PARTY_RELATIONSHIP`; Common IM change control; EHR Extract IM
  `X_VERSIONED_EHR_ACCESS`), BASE 1.3.0 (Architecture Overview, Security and
  Confidentiality), the BMM 2.4 P_BMM form of the vendored schemas;
  Regulation (EU) 2025/327 (EHDS) Art. 3(3), 4(2), 8, 9, 10, 11(5) and Annex
  II 3.2 and 3.4; Regulation (EU) 2016/679 (GDPR) Art. 6 and 9; ISO 22600 by
  its public scope; HL7 FHIR R4 and R5 `Consent`, IHE BPPC and IHE APPC
  (vendored under `docs/specs/fhir-consent/` and `docs/specs/ihe-consents/`
  by #5505, see [Sources](#sources-and-what-each-requires));
  the RBAC-as-demographic-data program (#5433 and its corrections), the admin
  registers (#5442, PR #5459), representatives (#5446) and the access log
  design (#5121, `docs/plans/access-log.md`).

This plan is the design and its rationale. Beside it,
`docs/plans/abac-openehr/antora/` holds the computable part of a draft
openEHR change, laid out as the access log draft is
(`docs/plans/access-log/antora/`):

- `tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`
  (moved there as the generator's input, #5506; on submission it goes to the
  RM repository's `computable/BMM/`): the draft package `org.openehr.rm.access_control` as a BMM 2.4 schema that includes
  `openehr_rm_1.2.0`, in the P_BMM JSON form of the vendored schemas under
  `tools/openehr-codegen/vendor/bmm/`;
- `antora/manifest-entry.json`: its entry for the RM component's
  `manifest.json`;
- `antora/PROVENANCE.md`: what the tree is and how it was checked.

The specification pages and the rendered class partials follow once the
owner has decided the model; writing them before the model is settled would
be text to throw away.

## The decision asked of the owner

**Recommendation: remove Cedar.** ABAC becomes facts held as openEHR
content, read by one fixed FerroEHR evaluator that knows only coded content
from FerroEHR's documented terminology, never an archetype id:

1. **Per EHR**, a concrete `ACCESS_CONTROL_SETTINGS` subtype,
   `ACCESS_DIRECTIVE_SETTINGS`, in `EHR_ACCESS.settings`: the default
   disposition, the gate-keepers, the subject's directives (consent permit
   and deny, EHDS Art. 8 restriction, EHDS Art. 10 opt-out, the scope of a
   delegation) and sensitivity labels on single objects. It is versioned with
   the EHR and travels in an EHR Extract.
2. **Per caller**, the 5.7.1 demographic model extended by three codes a
   `CAPABILITY` may carry (sensitivity clearance, permitted purpose, care
   setting) and by care relationships as ordinary `PARTY_RELATIONSHIP`s
   through care-team `GROUP`s.
3. **Per controller**, a new versioned `ACCESS_POLICY`: which kinds of data
   carry which sensitivity, whether clinical access needs a care relationship
   and a purpose, and a catalogue of named directives a patient can accept
   by reference (IHE BPPC).
4. **The evaluator** is fixed Rust: deny overrides permit, one level of
   exceptions, emergency override only where the directive allows it, and a
   `refused-*` code for every refusal. Cedar, the `cedar-policies` register
   of #5459, the `[authz.abac]` engine keys and the shipped example policies
   go, with no aid (greenfield).

Every decision a deployment can write in Cedar today is listed in
[Cedar today, decision by decision](#cedar-today-decision-by-decision) with
its openEHR form. The only loss is free composition of arbitrary boolean
conditions over token strings (a raw scope, a `sub`, a claim value). For a
CDR that loss is acceptable: those conditions bypass the party model this
program moves to, and the two that matter in practice (read-only scopes,
break-glass roles) are already decided by the SMART scope layer and by the
TOML break-glass floor. The remote PDP is the same second policy world over
HTTP; this plan recommends removing it too, as a separate ruling
([below](#the-remote-pdp)).

## What ABAC means here

**RBAC, as 5.7.1 ships it, answers "may this caller perform this kind of
operation at all?"** The answer depends only on who the caller is: the
`ROLE`s it performs and the operation codes their `CAPABILITY`s grant. A
laboratory relay may `commit_composition`; a migration client may
`import_ehr`; a professional may read clinical data. The answer is the same
for every patient.

**ABAC answers "may this caller do this, to this patient's data, now, for
this reason?"** The answer depends on attributes of the caller, of the data,
of the patient's record and of the request. Each row below is a decision
RBAC cannot make, because the same caller gets a different answer on a
different record or at a different moment.

| Attribute | The question it answers | Hospital example |
|---|---|---|
| Purpose of use | Why is the caller asking? | A nurse may read a ward patient's record for treatment (`TREAT`), but a quality-review export for the same nurse needs the purpose `HOPERAT` and the patient's research objection applies to it. |
| Patient consent and restriction | What did the patient allow or forbid? | A patient forbids her ex-partner, a GP at a neighbouring practice, from seeing her record: a deny directive naming his `PERSON`. A patient restricts her mental-health notes from every professional except her psychiatrist (EHDS Art. 8, with an exception). |
| Care relationship | Is the caller treating this patient now? | An internist on the cardiology ward reads the patients admitted there, and stops reading them two weeks after discharge (BASE Security, "Time-limitation of access"). A colleague on another ward does not read them at all, though she holds the same role. |
| Organisation | On whose behalf does the caller act? | A visiting consultant employed by the university hospital reads only the records this hospital shares with that organisation. A system client acts for its provider through its acts-for relationship. |
| Location and facility | Where is the care given? | The emergency department's devices write only for patients registered at the emergency department's facility code; a capability limits a client to one facility. |
| Time | Is the grant or the instruction in force? | A locum's capability runs from Monday 08:00 to Friday 18:00; a consent given for a clinical trial ends on the trial's end date. |
| Sensitivity | Is this kind of data more protected? | HIV results, psychiatric notes and genetic data carry a sensitivity label; only a caller whose capability clears that label, or a permit naming it, reads them. |
| Emergency | Is someone's life at stake? | In the emergency room a doctor declares the emergency purpose and reads the restricted allergy history; the access is marked as an emergency override and shown to the patient (EHDS Art. 11(5)). |

What stays RBAC: the operation codes (`import_ehr`, `commit_composition`,
`export_ehr_extract`), the client binding, the grant ceiling, the admin role
and the break-glass floor. ABAC adds conditions on top; it never grants an
operation the caller's capabilities do not grant.

## Sources and what each requires

### openEHR

- **RM EHR IM, `EHR_ACCESS`**
  (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.ehr.ehr_access.adoc`):
  "EHR-wide access control object. All access decisions to data in the EHR
  must be made in accordance with the policies and rules in this object."
  `settings: ACCESS_CONTROL_SETTINGS [0..1]` holds "a subtype of the type
  `ACCESS_CONTROL_SETTINGS`, allowing for the use of different access control
  schemes"; `scheme(): String` names it, with `Scheme_valid: not
  scheme.is_empty`.
- **RM EHR IM, `ACCESS_CONTROL_SETTINGS`**
  (`…org.openehr.rm.ehr.access_control_settings.adoc`): abstract, no
  attributes, "Intended to support multiple access control schemes.
  Currently implementation dependent."
- **RM EHR IM, EHR package, §EHR Access**
  (`docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc`): the settings
  include "default privacy policy, lists of identified accessors (individuals
  and groups) and exceptions to the default policies, each one identifying a
  particular Composition in the EHR. All changes to the EHR Access object are
  versioned via the normal mechanism". §EHR Creation: the EHR Access object is
  created and committed with the EHR.
- **BASE Architecture Overview, Security and Confidentiality**
  (`docs/specs/openehr/BASE/docs/architecture_overview/master07-security.adoc`):
  "data sharing must be controlled by patient consent" (§Privacy,
  Confidentiality and Consent); "the need for some access control to be
  specified in terms of categories or role types appears inescapable"
  (§Specifying Access Control); "the evaluation of a role category such as
  'care deliverer' into particular identities ... must be done in each care
  delivery environment, not in the EHR" (§The Problem of Roles); the access
  list, the gate-keeper, privacy levels whose definition "is defined by
  standards or agreements within jurisdictions of use" and "sensible
  defaults" (§Security Policy, Access Control); time-limitation of access
  bounded by the episode of care "plus some further time"; and §Access
  Control, Overview: "there is currently no published formal, proven model of
  access control for shared health information", so each model is "defined by
  a subtype of the class `ACCESS_CONTROL_SETTING`" and "the 'scheme' in use at
  any given time is always indicated in the EHR Access object".
- **RM EHR IM, Entry package, Standard Clinical Types of Data**
  (`docs/specs/openehr/RM/docs/ehr/master08-entry_package.adoc`): "Patient
  consents: recorded as instances of Admin_entry". The consent as a clinical
  document is record content; this design links a directive to it
  (`evidence`) and never replaces it.
- **RM Demographic IM** (`…org.openehr.rm.demographic.capability.adoc`,
  `role.adoc`, `party_relationship.adoc`, `party.adoc`): `CAPABILITY.credentials`
  are "the qualifications of the performer of the role for this capability";
  `ROLE.performer` is the actor; `PARTY_RELATIONSHIP` has `source`, `target`,
  `details`, `time_validity` (`DV_INTERVAL<DV_DATE>`) and `type() = name`;
  `PARTY.relationships` holds the relationships whose source the party is.
- **RM EHR Extract IM**: `X_VERSIONED_EHR_ACCESS` carries `EHR_ACCESS`
  versions in an extract, so settings in `EHR_ACCESS` move with the EHR.
- **ITS-REST 1.1.0** defines no `EHR_ACCESS` endpoint and mandates no
  authorization scheme (`Requests_and_responses.md` §Authentication and
  authorization). A change of `EHR_ACCESS` is a CONTRIBUTION.
- **What openEHR does not give:** no concrete settings class, no consent
  model, no rule language, no home for a controller-wide policy outside an EHR
  and the demographic service, and no authorization meaning for a
  `CAPABILITY`. CKM archetypes for consent or privacy are not vendored here
  and were not consulted; the build plan does not depend on any.

### Regulation and standards

- **EHDS Art. 8** (`docs/law/eu/ehds/text.html`): the right "to restrict the
  access of health professionals and healthcare providers to all or parts of
  their personal electronic health data"; "The fact that a natural person has
  restricted access ... shall not be visible to healthcare providers"; Member
  States set the rules and safeguards.
- **EHDS Art. 11(5)**: restricted content is not disclosed to the provider;
  "where necessary in order to protect the vital interests of the data
  subject, the healthcare provider or health professional may be granted
  access to the restricted electronic health data. Such cases shall be
  logged".
- **EHDS Art. 10**: a Member State's law may give a right to "opt out from the
  access ... through the electronic health data access services referred to in
  Articles 4 and 12", reversible; Art. 10(2) may admit a provider or
  professional for vital interests. Recital 55 lets a Member State make the
  opt-out absolute, with no emergency override.
- **EHDS Art. 4(2)**: proxy services let a person authorise others "for a
  limited or unlimited period and, if needed, for a specific purpose only",
  and let legal representatives act "in accordance with national law".
- **EHDS Art. 3(3)**: a Member State may delay the person's own access.
- **EHDS Art. 9 and Annex II 3.2**: the log records the provider and the
  natural person who accessed, the categories accessed, the time and the
  origins. **Annex II 3.4**: "access rights that take into account the
  origins and categories of electronic health data".
- **GDPR Art. 6(1) and 9(2)** (`docs/law/eu/gdpr/text.html`): care runs on
  Art. 9(2)(h) with Art. 6(1)(c) or (e), not on consent; Art. 9(2)(a)
  (explicit consent) and 9(2)(c) (vital interests where the subject cannot
  consent) are the other grounds a directive or an override names. A
  "consent directive" in care is therefore often a patient choice under
  national law, not GDPR consent, and the model records its legal ground
  explicitly (`legal_grounds`).
- **ISO 22600-1 and -2:2014**, by their public scope only
  (`docs/specs/iso-health-informatics/PROVENANCE.md`): policy agreements
  between domains, and formal policy, role, authorization and delegation
  models. The design names its three fact sets in those terms (a patient's
  policy per EHR, a controller's policy, the role and delegation model in the
  demographic service). Anything beyond the public scope is checkable only
  against the standard.
- **HL7 FHIR R4 and R5 `Consent`, IHE BPPC, IHE APPC:** vendored by #5505.
  FHIR R4 4.0.1 and R5 5.0.0 `Consent` (resource page, definitions,
  StructureDefinition, the bound value sets, the examples) under
  `docs/specs/fhir-consent/`; BPPC as ITI TF-1 chapter 19 and ITI TF-3
  section 5.1 of the HTML Technical Framework, and the APPC supplement Rev.
  1.4, under `docs/specs/ihe-consents/`. Every row of the
  [Mappings](#mappings) cites the section or element that grounds it.
- **OASIS XACML 2.0:** vendored by #5525. The core specification (OASIS
  Standard, 1 February 2005) under `docs/specs/xacml/`; APPC writes its
  consent documents in it (APPC TF-3 5.6.1). The APPC mapping cites its
  policy structure (§5.1, §5.22, §5.29, §5.30) and its combining algorithms
  (§B.10, Appendix C).

## Rules versus facts

The question is whether the CDR holds the rules (an expression language
inside archetyped content) or only the facts the rules read, with a small
fixed evaluator.

**Facts, with a fixed evaluator.** The reasons:

- **openEHR flexibility.** The 5.7.1 correction on #5433 fixed the principle:
  FerroEHR reads coded content, never archetype identity, so a deployment's
  own archetypes are first-class. A rule language inside content would make
  the meaning of a record depend on a program stored beside it; facts keep
  the meaning in the RM attributes and the terminology, which any archetype
  can constrain and any openEHR system can read.
- **Exchange.** A directive is data another system can read without running
  FerroEHR's engine. It maps to FHIR `Consent` provisions and to IHE APPC
  policies (below); a Cedar text maps to neither. An EHR Extract carries the
  `EHR_ACCESS` versions, so a patient's choices move with the record.
- **Performance.** Facts can be projected at commit into indexed columns and
  cached per EHR; a rule program has to be evaluated per request and per
  combination (today's Cedar fan-out is a cartesian product over patient,
  template, category and origin). Facts also let AQL filter in the one SQL
  statement a query runs, which a rule program cannot.
- **Auditability.** A refusal names the directive, the policy version and the
  code that decided it. A patient can read why, in their own terms. A Cedar
  decision names a policy id inside an operator's text.
- **Safety.** A fixed evaluator is tested once, exhaustively, and a refusal
  code is part of its contract. A deployment cannot write a rule that errors
  at evaluation, contradicts the EHDS invisibility rule, or opens data by
  accident through an OR-combined `permit`.

**What a fixed evaluator costs.** A deployment cannot invent a new kind of
condition. It can combine every attribute the model has, in every way the
model allows (scope, exception, effect, time, purpose, sensitivity), and a
new kind of condition is a new code or attribute in the next model version.
For a CDR this is the right trade: the conditions that matter are the ones
regulation and openEHR name, and those are all in the model.

## The model

### Overview

```
                 caller facts                      EHR facts                       controller facts
          (demographic service, 5.7.1)        (EHR_ACCESS, per EHR)          (ACCESS_POLICY, per controller)

 PERSON/AGENT --performs--> ROLE           EHR_ACCESS                       ACCESS_POLICY
     |                      +-CAPABILITY     settings: ACCESS_DIRECTIVE_      controller -> ORGANISATION
     |                         credentials:    SETTINGS                       classifications: data -> sensitivity
     |                          operation       default_access                care_relationship_required
     |                          clearance       gate_keepers                  relationship_grace
     |                          purpose         directives[] (consent,        purpose_required
     |                          care setting      restriction, opt-out,       catalogue[] (named directives)
     |                          facility          delegation) + exceptions
     +--member of--> GROUP (care team)         labels[] (object -> sensitivity)
                        ^
 patient PERSON --under care of--+              evidence -> ADMIN_ENTRY consent composition
     ^
     +--represents-- PERSON (parent, guardian, proxy; #5446)
```

Three fact sets, one evaluator, one terminology.

### Per EHR: `ACCESS_DIRECTIVE_SETTINGS` in `EHR_ACCESS.settings`

The concrete settings class of the scheme `access_directives`. It replaces
today's `FERROEHR_ACCESS_CONTROL_V1` (`app/ferroehr/src/service/ehr/access_types.rs`),
whose access list, gate-keeper and privacy levels it absorbs: the access list
becomes permit directives, the integer privacy levels become coded
sensitivity labels, and the gate-keeper stays. The BMM holds the full
documentation and invariants; this table is the summary.

| Class.attribute | Type | Meaning |
|---|---|---|
| `ACCESS_DIRECTIVE_SETTINGS.default_access` | `DV_CODED_TEXT` 1 | `open`, `restricted` or `consent_required` where no directive applies |
| `.gate_keepers` | `List<PARTY_PROXY>` 0..1 | who may commit the next `EHR_ACCESS` version (BASE Security, "Access control of access settings") |
| `.directives` | `List<ACCESS_DIRECTIVE>` 0..1 | the subject's instructions, current and past |
| `.labels` | `List<ACCESS_LABEL>` 0..1 | sensitivity labels on single versioned objects |
| `ACCESS_DIRECTIVE.kind` | `DV_CODED_TEXT` 1 | `consent`, `restriction`, `opt_out`, `delegation` |
| `.effect` | `DV_CODED_TEXT` 1 | `permit` or `deny` |
| `.applies_to` | `List<ACCESS_ACTOR_SCOPE>` | which callers; empty is every caller the kind can bind |
| `.operations` | `List<DV_CODED_TEXT>` | operation codes of the FerroEHR terminology; empty is every operation |
| `.purposes` | `List<DV_CODED_TEXT>` | purposes of use; empty is every purpose |
| `.data` | `List<ACCESS_DATA_SCOPE>` | which data; empty is the whole EHR, including future data |
| `.time_validity` | `DV_INTERVAL<DV_DATE_TIME>` | when in force, to the second |
| `.emergency_override` | `Boolean` 1 | whether a declared emergency lifts it |
| `.exceptions` | `List<ACCESS_DIRECTIVE>` | one level of narrower directives of the opposite effect |
| `.instructed_by`, `.instructed` | `PARTY_PROXY`, `DV_DATE_TIME` | who gave it and when |
| `.legal_grounds` | `List<DV_CODED_TEXT>` | GDPR, EHDS or national provision |
| `.evidence` | `List<DV_EHR_URI>` | the `ADMIN_ENTRY` holding the signed consent |
| `.identifiers` | `List<DV_IDENTIFIER>` | FHIR `Consent.identifier`, a BPPC or APPC policy id, a national registry reference |
| `.details` | `ITEM_STRUCTURE` | the deployment's own detail; the evaluator never reads it |
| `ACCESS_ACTOR_SCOPE.party` | `PARTY_REF` | a `PERSON` or `AGENT` matches itself; an `ORGANISATION` or `GROUP` matches whoever acts for it or is a member |
| `.role` | `DV_CODED_TEXT` | a functional role code the caller holds |
| `.relationship` | `DV_CODED_TEXT` | `care`, `representative` or `proxy` to the subject |
| `.caller_kind` | `DV_CODED_TEXT` | `professional`, `access_service`, `subject`, `representative`, `system` |
| `ACCESS_DATA_SCOPE.objects` | `List<OBJECT_REF>` | single versioned objects |
| `.templates`, `.archetypes` | `List<String>`, `List<ARCHETYPE_ID>` | the deployment's clinical templates and archetypes (data about the clinical record, which the deployment names freely) |
| `.categories` | `List<DV_CODED_TEXT>` | EHDS or national categories |
| `.sensitivities` | `List<DV_CODED_TEXT>` | sensitivity codes |
| `.origins` | `List<String>` | `system_id` of the originating system |
| `.recorded` | `DV_INTERVAL<DV_DATE_TIME>` | when the data were committed |
| `ACCESS_LABEL.target`, `.sensitivities`, `.reason` | `OBJECT_REF`, `List<DV_CODED_TEXT>` 1..*, `DV_TEXT` | a label on one object |

Why typed attributes here, where the 5.7.1 capabilities use coded elements
anywhere in `credentials`: `ACCESS_CONTROL_SETTINGS` is the one place the RM
leaves to the scheme, so the scheme can type its facts and give them
invariants, while `CAPABILITY.credentials` is a released `ITEM_STRUCTURE` that
FerroEHR may only read by code. Both stay archetype-agnostic: the evaluator
reads the typed attributes and the codes, and an archetype constrains them,
names them in the deployment's language and adds `details`.

### Per caller: the 5.7.1 demographic model, extended

The program of #5433 already binds the caller to an `AGENT` or `PERSON` and
reads its effective capabilities from the side table in one indexed read.
ABAC adds to what that read returns, all of it coded content in the same
terminology, at any path of the credentials tree:

- **Sensitivity clearance:** a `DV_CODED_TEXT` of the `access sensitivity
  clearance` group whose value is a sensitivity code (`PSY`, `HIV`,
  `GENETIC`, or the deployment's own). A psychiatrist's professional `ROLE`
  carries a capability clearing `PSY`.
- **Permitted purpose:** codes of the purpose-of-use terminology the caller
  may declare under this capability. A quality officer's capability permits
  `HOPERAT` and not `TREAT`.
- **Care setting and facility:** the facility scope 5.7.1 already reads, now
  applied to clinical reads too.
- **Care relationships:** the caller is a member of care-team `GROUP`s (a
  `PARTY_RELATIONSHIP` from the professional's `PERSON` to the `GROUP`, coded
  `member of`, with its own validity), and the patient's `PERSON` holds a
  `PARTY_RELATIONSHIP` `under care of` to the team `GROUP` or to the
  organisational unit, with the admission as its `time_validity` and the
  discharge time as a `DV_DATE_TIME` element of its `details` when a finer
  bound is needed. The hospital's ADT system writes them through the
  Demographic API as a system client with the `write_care_relationship`
  operation. This follows BASE Security, §The Problem of Roles: the care
  delivery environment evaluates "care deliverer", and the CDR reads the
  outcome.
- **Representation:** the parent, guardian and proxy relationships of #5446.
  A directive of kind `delegation` narrows what a proxy reads (EHDS Art.
  4(2)(a): "or part thereof ... for a specific purpose only").

Care relationships live in the demographic service because they are facts
about parties, they change with every admission, and they would otherwise
make every admission a new `EHR_ACCESS` version. They are stored on the
patient's `PERSON` (its source, RM `PARTY.relationships`) and the
membership on the professional's `PERSON`, so an admission versions one
party, and the roster versions the other, each at its own pace.

### Per controller: `ACCESS_POLICY`

Some facts hold for every EHR a controller keeps: "psychiatric notes are
`PSY`", "laboratory results fed by the lab are `LAB-INTERNAL`", "clinical
reads need a care relationship and a declared purpose", "the policy with OID
`2.16.840.1.113883.3.x.1` means: share with every provider in the region".
openEHR has no home for them: they are not about one EHR, and they are not a
party. Two options were weighed:

- **A new versioned root `ACCESS_POLICY`** in the proposal package, kept in
  its own `VERSIONED_ACCESS_POLICY`, committed in contributions with
  `AUDIT_DETAILS` like any versioned object. Clean, exchangeable, and the
  natural thing to propose to openEHR. It needs one FerroEHR extension
  resource, since ITS-REST has none. **Recommended.**
- **The controller `ORGANISATION`'s `details`.** Uses only released classes,
  but gives a party a meaning the RM does not give it and puts policy changes
  into the organisation's demographic history. Rejected.

`ACCESS_POLICY` holds `controller` (the `ORGANISATION`), `classifications`
(data scope to sensitivity codes), `care_relationship_required`,
`relationship_grace` (`DV_DURATION`), `purpose_required` and `catalogue`
(named directives a subject accepts by identifier). It holds no
break-glass and no default disposition floor: those stay in TOML (#5433),
and the policy can only tighten them.

### The terminology

The evaluator reads codes from one documented FerroEHR terminology, the one
#5433 introduces for operations (written `ferroehr-operations` on that
issue). This design adds groups to it; each code is a rubric until codes are
assigned, as in the access log draft:

| Group | Codes |
|---|---|
| `access default` | `open`, `restricted`, `consent_required` |
| `access directive kind` | `consent`, `restriction`, `opt_out`, `delegation` |
| `access effect` | `permit`, `deny` |
| `access relationship` | `care`, `representative`, `proxy` |
| `access caller kind` | `professional`, `access_service`, `subject`, `representative`, `system` |
| `access sensitivity clearance` | the element whose value is a sensitivity code |
| `care relationship` (demographic) | `member_of`, `under_care_of` |
| operation (from #5433) | adds `read`, `write`, `export`, `query` as directive-level operation classes, and `write_care_relationship`, `write_access_policy` as grants |

Sensitivity, category and purpose codes are not FerroEHR's: they come from
the terminology the jurisdiction names (HL7 v3 `ActCode` sensitivity and
`Confidentiality`, the EHDS categories, the HL7 `PurposeOfUse` codes the
IUA token states). The evaluator compares them by terminology id and code.
In the BMM the invariants name `Terminology_id_access_control`: `openehr` if
openEHR adopts the groups, FerroEHR's terminology until then.

### Archetypes a deployment writes

Nothing below is recognised by id. Each is an example of what a deployment
models with its own archetypes; FerroEHR ships them as defaults, the way it
ships the 5.7.1 policy definitions.

- `openEHR-EHR-EHR_ACCESS.access_directives.v1`: constrains `settings` to
  `ACCESS_DIRECTIVE_SETTINGS`, names the directives in the deployment's
  language (`at10` "Mental health restriction", `at11` "Research objection",
  `at12` "Regional sharing consent"), fixes the purpose and sensitivity value
  sets the hospital uses, and adds `details` such as "consent form version"
  and "witness".

  ```
  EHR_ACCESS[id1] matches {
      settings matches {
          ACCESS_DIRECTIVE_SETTINGS[id2] matches {
              default_access matches { DV_CODED_TEXT[id3] }   -- bound to the access default group
              directives cardinality matches {0..*} matches {
                  ACCESS_DIRECTIVE[id4] occurrences matches {0..*} matches {
                      kind matches { DV_CODED_TEXT[id5] }
                      effect matches { DV_CODED_TEXT[id6] }
                      data matches { ACCESS_DATA_SCOPE[id7] }
                      details matches { ITEM_TREE[id8] }
                  }
              }
          }
      }
  }
  ```
- `openEHR-EHR-COMPOSITION.consent_record.v1` with an `ADMIN_ENTRY` of the
  patient's decision and the scanned or signed form: the clinical record of
  the instruction, which a directive's `evidence` points to (RM Entry
  package: consents are `ADMIN_ENTRY`).
- `openEHR-DEMOGRAPHIC-GROUP.care_team.v1` and
  `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v1`, coded with the
  `care relationship` group.
- A professional `ROLE` with a `CAPABILITY` whose credentials carry a
  sensitivity clearance element: `openEHR-DEMOGRAPHIC-ROLE.professional.v1`
  from the default demographic set gains an optional element.
- `openEHR-EHR-ACCESS_POLICY.hospital.v1`: the controller policy, with the
  hospital's classification rules and its BPPC catalogue.

A deployment that already models consent its own way keeps its archetypes;
it binds the coded attributes to its value sets and is read the same way.

### The decision procedure

FerroEHR's own design; no specification gives one. It runs after
authentication, the 5.7.1 capability check and the SMART scope layer, per EHR
the operation reaches, over the cached facts. The first step that refuses
decides.

1. **Opt-out** (EHDS Art. 10). An `opt_out` deny directive in force binds
   callers of kind `access_service`. Unless its `emergency_override` holds and
   the caller declares an emergency purpose and does not act for the
   subject, the EHR reads as absent: `404`, logged as
   `refused-opt-out`.
2. **Restriction** (EHDS Art. 8). A `restriction` deny directive in force
   binds callers of kind `professional` (and the systems that act for a
   provider). Data it covers read as absent (`404` on a named object, left
   out of queries and exports), logged as `refused-restricted`. An
   `emergency_override` with a declared emergency purpose and the break-glass
   role lifts it and marks the access (Art. 11(5)).
3. **Consent deny.** Any other deny directive that applies, with no
   exception that applies, refuses: `403 refused-consent-denied`.
4. **Sensitivity.** Data whose labels (from `ACCESS_LABEL` and the
   controller's classifications) are not all cleared by the caller's
   capabilities, and not named by an applying permit directive, are refused:
   `403 refused-sensitivity` on a named object, left out of a query.
5. **Care relationship.** Where `ACCESS_POLICY.care_relationship_required`
   holds and the caller is a professional, the caller needs a live care
   relationship to the subject (membership of a team the patient is under
   the care of, within its validity plus `relationship_grace`), a permit
   directive naming it, or a gate-keeper role: else `403
   refused-no-care-relationship`.
6. **Purpose.** Where `purpose_required` holds, a request without a declared
   purpose is refused `403 refused-purpose-missing`; a purpose the caller's
   capabilities do not permit is refused `403 refused-purpose-not-permitted`.
7. **Default disposition.** `open` admits. `restricted` admits a caller that
   an applying permit directive, a gate-keeper entry or a live care
   relationship names. `consent_required` admits only an applying permit
   directive or the subject: else `403 refused-consent-required`. An EHR with
   no settings takes `[authz.rbac] ehr_access_default`, as today.

The subject reading their own record skips steps 3 to 7 (EHDS Art. 3), and is
held only by the access delay register (Art. 3(3)). A representative is held
to the subject's directives and to any `delegation` directive that names
them. Every combination of a set attribute is evaluated, as today; a query
filters rows by the same facts in SQL ([AQL](#aql)).

### Worked examples

- **The ex-partner.** Ms A's `EHR_ACCESS` holds a `consent` deny directive,
  `applies_to.party` = Dr B's `PERSON`, no data scope. Dr B is on call in the
  emergency department and opens her record: step 3 refuses `403
  refused-consent-denied`; the directive's `emergency_override` is true, so
  if he declares `ETREAT` with the break-glass role, step 3 passes and the
  access log marks the override, which Ms A sees.
- **Mental health, except the psychiatrist.** A `restriction` deny directive
  with `data.categories` = mental health and one exception, a `permit` with
  `applies_to.party` = Dr C. A ward nurse's query leaves the notes out with
  no trace of a restriction; Dr C reads them.
- **The ward.** The hospital policy sets `care_relationship_required` and a
  14-day grace. Dr D is a member of the cardiology team; Mr E is under the
  care of cardiology from 3 to 10 November. Dr D reads Mr E's record until 24
  November; on the 25th step 5 refuses. Dr F on nephrology never reads it,
  unless Mr E is transferred or a consult adds her team.
- **HIV results.** The policy classifies the template `lab_hiv.v1` as `HIV`.
  An infectious-disease specialist's capability clears `HIV`; a dermatologist
  reading the same patient sees the rest of the record, and the HIV results
  are left out.
- **Regional sharing (BPPC).** The policy catalogue holds a permit directive
  identified by the region's opt-in policy (its BPPC code and code system),
  applying to callers acting for any `ORGANISATION` of the regional `GROUP`.
  A patient who signs it gets a directive copied from the catalogue entry
  with the signed form as `evidence`; `default_access` stays
  `consent_required` for everyone else.

### Per-request resolution with no added transactions

The perf gate counts transactions per request; #5459 measured that a
per-request read of a register costs 6.5 to 7.5. The design adds none:

- **Caller facts** come from the 5.7.1 side-table read the request already
  makes, extended with care-team ids, clearances and permitted purposes.
- **EHR facts** are projected at the commit of an `EHR_ACCESS` version, in
  its transaction, into a side table (`access_directive`: one row per
  directive and exception, with indexed columns for kind, effect, actor
  party, role, relationship, sensitivity, category and validity), and the
  patient's `under care of` team ids are projected at the commit of the
  patient's party into the binding side table. Each replica caches the
  compiled facts per EHR and drops an entry on a `LISTEN`/`NOTIFY` message
  sent from the committing transaction, the mechanism #5459 uses for the
  registers. This replaces today's few-second expiry of the `EHR_ACCESS`
  cache, which can leave a revoked consent in force on another replica for
  that long. While a listener is down the replica reads per request, and a
  store that cannot answer is `503`, as for the registers.
- **Controller facts** (`ACCESS_POLICY`) are held in memory with the same
  invalidation.
- **The decision** is a pure function over those three inputs.

### AQL

A query applies the same procedure as a condition of its one SQL statement,
the way the policy-party filter of #5450 does: a join of the candidate
versioned objects against `access_directive`, the label and classification
projections and the caller's facts, so `COUNT`, `LIMIT` and paging count only
rows the caller may read, and nothing a caller may not read leaves the
database. Today's query scope loads every EHR's settings into one cached
index; a per-row projection replaces it and scales with the number of
directives, not the number of EHRs.

### Audit and explanation

- **The access record** (EHDS Annex II 3.2) gains the grounds the decision
  rested on, in the shape the access log draft already defines
  (`ACCESS_AUTHORISATION`, `docs/plans/access-log.md`): the capability and
  role (`role`), the care relationship (`treatment relationship`), each
  directive with its `EHR_ACCESS` version uid (`consent decision`), the
  `ACCESS_POLICY` version, and an emergency override. The `ehds-logging`
  record carries the same as its authorisation reference.
- **A refusal** carries its `refused-*` code in the problem body of a `403`,
  and in the access record of every refusal, including the `404`s of steps 1
  and 2, whose response says nothing (EHDS Art. 8 third paragraph). The new
  codes: `refused-consent-denied`, `refused-consent-required`,
  `refused-sensitivity`, `refused-no-care-relationship`,
  `refused-purpose-missing`, `refused-purpose-not-permitted`, and, in the log
  only, `refused-restricted` and `refused-opt-out`.
- **The patient** reads, through the access log, every access with its
  grounds and every emergency override (EHDS Art. 9 and 11(5)).

### Who reads and writes the settings

- A new `EHR_ACCESS` version is committed by a gate-keeper, the subject
  (`PARTY_SELF`) or a representative where the gate-keepers name them, or
  `admin_role` acting on the subject's instruction (the controller's route,
  as the restriction and opt-out admin routes are today). Everyone else is
  refused `403`.
- **Reading** the settings is limited to the same callers, because a
  restriction must not be visible to providers (EHDS Art. 8). The versioned
  `EHR_ACCESS` routes answer `404` to anyone else, as an unknown one would.
- Every write is checked against the BMM invariants and the terminology
  groups before it is stored (strict, as every RM write is), and refused whole
  with `400` naming the first defect.

### Versioning, exchange and emergency

- Every change is an `EHR_ACCESS` version in a contribution, so the history
  of a patient's choices is the version history, and a revoked consent stays
  visible as a past version (BASE Security, "Audit trailing").
- An EHR Extract carries the `EHR_ACCESS` versions (`X_VERSIONED_EHR_ACCESS`).
  An importing FerroEHR applies them as they are; another system that does
  not know the scheme sees an unknown `scheme()` and keeps the object as
  data. `ACCESS_POLICY` does not travel: it is the controller's, not the
  patient's.
- Break-glass stays in TOML (`[authz.rbac] emergency_access_role`, `[audit]
  emergency_purpose_codes`), so it works when the store does not answer;
  whether a directive yields to it is the directive's own
  `emergency_override`, which a Member State's absolute opt-out sets false
  (EHDS recital 55).

## The BMM

`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`:

- **Schema:** `schema_name` `access_control`, `rm_release` `0.1.0`, lifecycle
  `draft`, includes `openehr_rm_1.2.0`, author FerroEHR (#5487), the same
  header the access log draft carries.
- **Package:** `org.openehr.rm.access_control`, named as an openEHR RM
  package so it can be submitted as one; FerroEHR emits it as a draft module,
  never into the published `openehr-*` crates.
- **Classes:** `ACCESS_DIRECTIVE_SETTINGS` (inherits
  `ACCESS_CONTROL_SETTINGS`), `ACCESS_DIRECTIVE` (`LOCATABLE`),
  `ACCESS_ACTOR_SCOPE`, `ACCESS_DATA_SCOPE`, `ACCESS_LABEL`, `ACCESS_POLICY`
  (`LOCATABLE`, archetype root), `ACCESS_CLASSIFICATION` (`LOCATABLE`) and
  `VERSIONED_ACCESS_POLICY` (`VERSIONED_OBJECT`).
- **Invariants:** the terminology group of every coded attribute; a
  restriction is a deny; an opt-out is a whole-record deny and at most one per
  EHR; exceptions have the opposite effect and no exceptions of their own; an
  actor or data scope sets at least one attribute; a classification names no
  single object; catalogue entries are identified; every optional list is
  absent or non-empty (the `LOCATABLE.Links_valid` family).

**Checked with the repository's BMM loader.** The draft-module machinery of
#5124 (`tools/openehr-codegen/src/plan/drafts.rs` on
`feat/access-log-draft-types`, not yet on `main`) was run in a scratch copy
with this schema added as a second draft row:

- `compose_draft` loads it with `BmmSchema::parse_json`, checks its
  `includes` against the declared `openehr_rm_1.2.0`, merges BASE 1.3.0 and
  RM 1.2.0, and refuses a type that resolves nowhere. It composed. A probe
  that renamed one type to an undefined class was refused, naming it.
- `emit-drafts` rendered the module (`access_control_draft`, every class and
  its canonical-JSON codec) and `cargo check -p ferroehr` compiled it. See
  `antora/PROVENANCE.md` for the run.
- One generator finding for the build: because the draft adds subtypes of
  `LOCATABLE`, `VERSIONED_OBJECT` and `ACCESS_CONTROL_SETTINGS`, the
  re-emission closure re-emits the RM classes whose slots now admit them
  (the EHR Extract content items among them) into the draft module. That is
  the generator's documented behaviour; the build issue decides whether the
  draft module keeps those re-emitted classes or the RM generation gains an
  extension point for draft subtypes of `ACCESS_CONTROL_SETTINGS`.

**Decided in #5506.** The RM generation already has that extension point:
it types `EHR_ACCESS.settings` with the verbatim `OpenSubtype` carrier
(`plan::overrides::open_extension_point`), so no RM class that holds the
settings is re-emitted. The draft module keeps the classes re-emitted for
`LOCATABLE` and `VERSIONED_OBJECT`: `LOCATABLE`, `VERSIONED_OBJECT` and the
EHR Extract classes down to `GENERIC_CONTENT_ITEM`, whose `item` is any
`LOCATABLE` (RM EHR Extract IM). They are the downstream seam the codegen
rule asks for, and an upstream crate never gains a downstream variant. The
generator also opens the settings seam: for a draft subtype of an open
extension point, `emit-drafts` writes `open_seam.rs` with the two `TryFrom`
conversions between `ACCESS_DIRECTIVE_SETTINGS` and the carrier, through the
canonical-JSON codec, which refuses a carrier of another scheme.

## Mappings

### FHIR `Consent`, R4 and R5, both ways

Checked against the vendored R4 (4.0.1) and R5 (5.0.0) texts under
`docs/specs/fhir-consent/` (#5505): the resource page (`consent.html`), the
formal definitions (`consent-definitions.html`, cited below by element), the
StructureDefinition (`consent.profile.json`, for cardinality and bindings),
the bound value sets and the published examples.

**The structure is a base policy with exceptions.** Both releases state it in
§6.2.3 Background and Context: R4 "a base policy (represented as
Consent.policy/Consent.policyRule) which is either opt-in or opt-out,
followed by a listing of exceptions to that policy (represented as
Consent.provision(s))"; R5 "a base policy (represented as Consent.decision)
which is either permit or deny, followed by a listing of exceptions". R4
`Consent.provision.type` is "Not permitted in root rule, required in all
nested rules", and the root `Consent.provision` is "An exception to the base
policy". R5 has no `provision.type`: `decision` is the default and every
provision is an exception to its parent. Several published examples of
both releases use three levels: the base, one exception carrying the scope, and
exceptions nested in it (`consent-example-Emergency.json`,
`consent-example-pkb.json`, `consent-example-smartonfhir.json`). That is the
shape of a directive with a scope and exceptions, with the opposite effect as
the base. The mapping is:

- **Export.** A directive with empty `applies_to`, `operations`, `purposes`
  and `data` is the base (R4 `policyRule`, R5 `decision`), and its
  `exceptions` are the exception provisions. A directive with a scope is
  written as the base of the opposite effect, its scope as the one exception
  provision, and its own exceptions nested inside that one. The R4 base is
  `policyRule` `OPTIN` (permit) or `OPTOUT` (deny) of v3 ActCode, the codes
  the R4 examples use (`consent-example-notThem.json`,
  `consent-example-Emergency.json`) under the extensible
  ConsentPolicyRuleCodes binding. A scoped directive's `Consent` states the
  opposite effect outside the scope, where FerroEHR decides by
  `default_access` and the other directives: a loss to FHIR.
- **Import.** A `Consent` whose exception provisions carry no nested
  provision is one directive: the base's effect over the whole EHR, the
  exception provisions as its `exceptions`. A `Consent` whose exception
  provisions nest further gives one directive per exception provision: the
  exception's effect, its conditions as the scope, the nested provisions as
  its `exceptions`. Its base then adds a whole-EHR permit directive when it
  permits (deny-overrides keeps every scoped deny in force). When it denies,
  it adds nothing if the EHR's `default_access` is not `open`, and the
  `Consent` is refused if it is, since a whole-EHR deny directive would
  override the scoped permits. A provision nested a level deeper is refused
  with the element named. A scoped directive therefore comes back from its
  own export with its base as a second fact; the owner decides whether that
  stands or the export of scoped directives stays one way.
- **Codes the import reads.** An R4 `policyRule` other than `OPTIN` or
  `OPTOUT` is read as an `ACCESS_POLICY.catalogue` identifier and refused
  when the catalogue does not hold it. An R5 `Consent` with provisions and no
  `decision` is refused: it has no base. A nested R4 `type` equal to its
  parent's effect is refused, since it would not be an exception; an absent
  one (`consent-example-pkb.json` nests eleven) is read as the opposite of
  its parent, the way R5 reads every provision.

The text fixes the base and the exceptions; how a base maps onto
`default_access` and how a three-level `Consent` splits into directives is
FerroEHR's own design, since FHIR leaves enforcement out of scope (§6.2.3:
"The specification of these details is not in scope for the Consent
resource").

The R4 examples do not read the root provision one way:
`consent-example-Out.json` pairs `OPTOUT` with a root provision whose
narrative withholds data, while `consent-example-Emergency.json` pairs
`OPTOUT` with a root provision that permits. The import follows the
definition (the root provision is an exception to the base), and the
round-trip tests name `consent-example-Out.json` as the R4 example whose
narrative reads the other way. `consent-example-notOrg.json` carries a root
`type` the definition does not permit; the import accepts a root `type` that
equals the exception's effect and refuses one that contradicts it. Of the
vendored examples, `consent-example-notThis.json` (both releases) is refused
for its `provision.data.meaning` `related`; every other one imports under
these rules.

| `ACCESS_DIRECTIVE` | FHIR R4 `Consent` | FHIR R5 `Consent` | Ground in the vendored text |
|---|---|---|---|
| in force in the current `EHR_ACCESS` version | `status` `active`; a directive removed in a later version `inactive` | `status` `active` / `inactive` | `Consent.status`, required ConsentState: `inactive` "The consent is terminated or replaced" |
| `kind` | `scope` `patient-privacy`; `category` LOINC `59284-0` Patient Consent (`57016-8` Privacy policy acknowledgement Document for a directive copied from a BPPC catalogue entry), with the FerroEHR kind code as a second coding | `category`: the FerroEHR kind code, LOINC `59284-0` beside it | R4 `Consent.scope` 1..1, extensible, "This list is now extensible"; R4 `Consent.category` 1..*, extensible ConsentCategoryCodes, which holds `59284-0` and `57016-8` and no code for a restriction, an opt-out or a delegation; R5 `Consent.category` 0..*, example binding |
| EHR subject | `patient` | `subject` | R4 `Consent.patient`, required by ppc-2 when the scope is privacy; R5 `Consent.subject` Reference(Patient \| Practitioner \| Group) |
| `instructed` | no element; `dateTime` takes the commit time of the `EHR_ACCESS` version, and `instructed` goes in the FerroEHR extension | `date`, the day only | R4 `Consent.dateTime`: "not the time of the original consent, but the time that this statement was made or derived"; R5 `Consent.date` is a `date`, "Date the consent instance was agreed to" |
| `instructed_by` | `performer` | `grantor` | R4 `Consent.performer` is "Either the Grantor ... or the Grantee", so an R4 import cannot tell the two apart; R5 `Consent.grantor` |
| controller (from `ACCESS_POLICY`) | `organization` | `manager`, `controller` | R4 `Consent.organization` "manages the consent"; R5 `Consent.manager`, `Consent.controller` "controls/enforces the access" |
| `evidence` | `sourceReference`, one item | `sourceReference` | R4 `Consent.source[x]` 0..1; R5 `Consent.sourceReference` 0..* (DocumentReference among its targets) |
| `legal_grounds` | `policy`, one per ground (`uri` the provision, `authority` the issuing body) | `regulatoryBasis` | R4 `Consent.policy` 0..*, `uri`; `policyRule` 0..1 carries the base (§6.2.3, §6.2.5), and ppc-1 needs `policy` or `policyRule`; R5 `Consent.regulatoryBasis` 0..*, example ConsentPolicyRuleCodes |
| the catalogue entry a directive was copied from | `policy.uri`, the policy identifier (`urn:oid:` for a BPPC policy) | `policyBasis.url`, the URL of the entry on the `ACCESS_POLICY` resource | R4 `Consent.policy.uri` "Specific policy covered by this consent"; R5 `Consent.policyBasis.url` "should be dereferencable to a computable policy" |
| `identifiers` | `identifier` | `identifier` | `Consent.identifier` 0..* in both |
| `effect` | the base or the exception, as above | the base or the exception, as above | §6.2.3 and `Consent.provision.type` (R4), `Consent.decision` (R5) |
| `time_validity` | root `provision.period` | `period` | R4 has no `Consent.period`; R4 `provision.period` "The timeframe in this rule is valid"; R5 `Consent.period` covers "all provisions unless specified in that provision" |
| `applies_to.party` | `provision.actor.reference`, with `actor.role` a participation code (`PRCP` in the examples) | `provision.actor.reference`, `actor.role` optional; `grantee` repeats the parties for search | `provision.actor.role` is "How the individual is involved", bound to SecurityRoleType (R4, 1..1) and ParticipationRoleType (R5, 0..1), never the caller's functional role; R5 `Consent.grantee`: "for ease of search only" |
| `applies_to.role` | `provision.actor.reference` to a `Group` of the actors holding the role | the same | `provision.actor.reference`: "use group to identify a set of actors by some property they share (e.g. 'admitting officers')" |
| `operations` | `provision.action` | `provision.action` | example ConsentActionCodes; "the only action in the understood and tested scope of this resource is 'read'" |
| `purposes` | `provision.purpose` | `provision.purpose` | extensible PurposeOfUse, `Coding` |
| `data.sensitivities` | `provision.securityLabel` | `provision.securityLabel` | "If the consent specifies a security label of 'R' then it applies to all resources that are labeled 'R' or lower"; for other labels "subsumption logic applies" |
| `data.categories` | `provision.code` | `provision.code` | "If this code is found in an instance, then the rule applies" |
| `data.templates` | `provision.class` | `provision.documentType` | R4 `provision.class`: "a FHIR resource type, a profile on a type, or a CDA document, or some other type"; R5 `provision.documentType` "some other type that indicates what sort of information"; R5 `provision.resourceType` is bound extensibly to FHIR resource types and takes no openEHR template |
| `data.recorded` | `provision.dataPeriod` | `provision.dataPeriod` | "Clinical or Operational Relevant period of time that bounds the data"; the commit time is the operational period, so an imported clinical period is read as a commit-time bound |
| `data.objects` | `provision.data` (`meaning` `instance`) | `provision.data` (`meaning` `instance`) | required ConsentDataMeaning: `instance` "applies directly to the instance of the resource" |
| `exceptions` | `provision` with `type`, inside the exception | `provision`, inside the exception | R4 nested `provision.type` is required; R5 `provision.provision` "Provisions which provide exceptions to the base provision or subprovisions" |

Losses, in each direction:

- **From FHIR:** a provision nested deeper than the three levels above;
  `provision.data` with
  `meaning` `related`, `dependents` or `authoredby` (ConsentDataMeaning has
  these four codes); R5 `provision.expression`; R5 `policyBasis` that names a
  computable policy other than a catalogue entry; an R4 `policyRule` the
  catalogue does not hold; an actor reference to a resource with no party in
  the demographic service. An import refuses the `Consent` and names the
  element, never drops it. `verification` maps to `details` only. A
  `securityLabel` from the confidentiality system covers its level and every
  lower one, so the import maps it to every sensitivity code at or below
  it.
- **To FHIR:** `emergency_override`, `relationship` and `caller_kind` have no
  core element; they go in a FerroEHR extension. `origins` maps to no core
  element either. R4 loses `instructed` to the extension and every
  `evidence` item after the first; R5 keeps the day of `instructed` only. A
  scoped directive states the opposite effect outside its scope (above).
  The model's one level of exceptions holds every shape the vendored
  examples use.
- **R4 `Consent.scope`** is bound extensibly to the four codes `adr`,
  `research`, `patient-privacy` and `treatment`; every directive is
  `patient-privacy`, and the kinds are told apart by `category`.

### IHE BPPC

Checked against ITI TF-1 chapter 19 and ITI TF-3 section 5.1 under
`docs/specs/ihe-consents/profiles.ihe.net/ITI/TF/` (#5505).

BPPC records that a patient acknowledged a privacy policy the policy domain
publishes; it carries no structured rule ("the content of the policy is human
readable text, and that we provide no structured or coded way to interpret",
TF-1 19.1.1). The acknowledgement is a CDA R2 document (TF-3 5.1.2, template
`1.3.6.1.4.1.19376.1.5.3.1.1.7`; with a scanned part, 5.1.3, template
`.1.7.1` on XDS-SD) with one `documentationOf/serviceEvent` per policy
acknowledged (5.1.2.2.3). Each `serviceEvent/code` names the policy: `code`
is the Patient Privacy Policy Identifier and `codeSystem` the policy domain's
code system (5.1.2.2.6). Each `serviceEvent/effectiveTime` gives the range
over which the acknowledgement holds: `low` required, `high` optional, the
same in every service event of one document (5.1.2.2.7).

FerroEHR maps each service event to the `ACCESS_POLICY.catalogue` entry with
that `code` in that `codeSystem`: the entry's directive is copied into the
patient's `EHR_ACCESS` with `effectiveTime` as its `time_validity`, the
policy code in `identifiers`, and the BPPC document (stored as a
composition) as `evidence`. A document that acknowledges several policies
gives one directive per service event, all with the same `evidence`. A
policy the catalogue does not hold is refused, since FerroEHR cannot know
what it means. An opt-out policy (TF-1 19.1.1.2) is a catalogue entry whose
directive denies. Where the domain runs on implied consent, no
acknowledgement exists (TF-1 19.1.1, 19.6.1) and `default_access` carries
that rule.

Export writes, for the directives copied from catalogue entries, one
acknowledgement document per distinct `time_validity` ("If different
effectiveTimes are needed then different documents should be generated",
5.1.2.2.7), with the text description, the policy codes and the time range
the content module requires (5.1.2.2), LOINC `57016-8` and format code
`urn:ihe:iti:bppc:2007` (5.1.2.1.1). No loss for policies in the catalogue.
BPPC defines no revocation: a directive removed in a later version is
withdrawn by the document sharing infrastructure, from which a consumer
reads only "Approved" acknowledgements (TF-1 19.6.4).

### IHE APPC

Checked against the APPC supplement, Rev. 1.4 (Trial Implementation, July 2,
2021), under `docs/specs/ihe-consents/www.ihe.net/` (#5505). APPC builds on
XACML 2.0 (TF-3 5.6.1); the statements below about XACML's own structure
cite the core specification under `docs/specs/xacml/` (#5525) as "XACML"
with its section.

A Privacy Consent Document is an XACML 2.0 document with exactly one root
`PolicySet`, whose `PolicySetId` is a URN-encoded UUID, which holds a
`Description`, and which matches the patient by
`urn:ihe:iti:ser:2016:patient-id` (TF-3 5.6.2.1.1, 5.6.2.1.1.1). The patient
match maps to the EHR subject. The root may reference the policy domain's
foundational policies by `PolicySetIdReference` or `PolicyIdReference`
instead of carrying rules; "How external references are resolved is out of
scope for this profile" (5.6.2.1.1), and the profile's own example
(5.6.2.1.1.2) is such a reference narrowed by a subject `Target`. FerroEHR
resolves a reference against `ACCESS_POLICY.catalogue`, as for BPPC, and
refuses one the catalogue does not hold. When a BPPC document is the human
readable form, the Root Policy Set ID equals the BPPC policy identifier
(5.6.2.1.1.1).

A policy the document carries maps to one directive. XACML places `Effect`
on `Rule` alone: `RuleType` requires the attribute and `EffectType` takes
`Permit` or `Deny` (XACML 5.29, 5.30), while `PolicyType` and
`PolicySetType` carry no effect (5.22, 5.1). The directive's `effect` is
therefore the `Effect` of the policy's base rule, the rule whose target is
the policy's own (a rule with no `Target` takes the policy's, 5.29). A rule
of the opposite effect whose target narrows the base maps to an exception,
provided the policy's `RuleCombiningAlgId` lets that rule decide where both
apply, as an exception does; anything else is refused. XACML 5.22 makes
`RuleCombiningAlgId` required on every `Policy`, so the pairing is always
stated:

- deny-overrides (C.1) or ordered-deny-overrides (C.2, the same algorithm
  evaluated in document order): a `Permit` base with `Deny` exceptions;
- permit-overrides (C.3) or ordered-permit-overrides (C.4): a `Deny` base
  with `Permit` exceptions;
- first-applicable (C.5): either base, when every exception rule precedes
  the base rule, since the first rule whose target and condition hold
  decides.

Under deny-overrides a `Permit` rule never decides where a `Deny` rule also
applies (C.1), so a `Deny` base with `Permit` rules under it is refused with
the policy named, and so is the mirror case under permit-overrides. A
policy with a single rule maps whatever its rule-combining algorithm. The
attributes map by the profile's tables:

| APPC attribute (TF-3 section) | `ACCESS_DIRECTIVE` |
|---|---|
| Subject: User ID, string (5.6.2.1.4.1); NPI and Other Provider Identifier, `II` (5.6.2.1.4.5, .8) | `applies_to.party`, a `PERSON` found by identifier |
| Subject: User Organization ID, `anyURI` (5.6.2.1.4.3) | `applies_to.party`, an `ORGANISATION` |
| Subject: User Role, `CV` (5.6.2.1.4.6) | `applies_to.role` |
| Subject: Purpose Of Use, `CV` (5.6.2.1.4.7) | `purposes` |
| Resource: Patient ID (5.6.2.1.5.1.5) | the EHR subject |
| Resource: Confidentiality Code (5.6.2.1.5.2.2) | `data.sensitivities` |
| Resource: Class Code, Type Code (5.6.2.1.5.2.1, .13) | `data.categories` |
| Resource: Author Institution ID, Source System ID (5.6.2.1.5.1.1, .6) | `data.origins` |
| Resource: Creation Time (5.6.2.1.5.2.3) | `data.recorded` |
| Resource: Document Unique ID, Reference ID List (5.6.2.1.5.2.14, .9) | `data.objects`, where the identifier names an object the EHR holds |
| Action: `action-id`, an IHE transaction URI (5.6.2.1.6.1) | `operations`, through a fixed table from transaction to FerroEHR operation (own design) |
| Document entry `serviceStartTime`, `serviceStopTime` (5.6.2.2.1.6, .7) | `time_validity` |

APPC defines no environment attribute ("No additional constraints",
5.6.2.1.7), so `time_validity` comes from the document entry's service
times. A time condition inside the policy that is a plain interval narrows
it; any other condition is refused. User Organization (a plain-text name),
User Home Community ID, Author Person ID, Event Code, Healthcare Facility
Type Code, Practice Setting Code, Service Start and Stop Time of a document,
and the Folder and SubmissionSet attributes have no form in `data` or
`applies_to` and are refused on import with the attribute named.

APPC fixes no combining algorithm: it lists "Should the profile define a
policy-combining and a rule-combining algorithm?" as open issue APPC-7, and
its example uses
`urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:deny-overrides`
(5.6.2.1.1.2). XACML defines the two kinds apart: a `PolicySet` names a
policy-combining algorithm in `PolicyCombiningAlgId` and a `Policy` a
rule-combining algorithm in `RuleCombiningAlgId`, both required (5.1,
5.22). Its identifiers (B.10), all of which a conforming implementation
must support (10.2.3), are `deny-overrides`, `permit-overrides` and
`first-applicable` in both kinds under
`urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:` and
`urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:`,
`only-one-applicable` for policy sets alone under the second, and
`ordered-deny-overrides` and `ordered-permit-overrides` in both kinds under
the `urn:oasis:names:tc:xacml:1.1:` prefixes. XACML 2.0 adds no `2.0`
combining identifier.

Directives combine by deny-overrides across an EHR (step 3 of the decision
procedure), which is the policy-combining deny-overrides of XACML C.1: any
policy that evaluates to `Deny` decides. A policy set whose
`PolicyCombiningAlgId` is deny-overrides or ordered-deny-overrides maps
whole; permit-overrides, ordered-permit-overrides, first-applicable and
only-one-applicable (C.3 to C.6) are refused on import with the element
named. A policy's `RuleCombiningAlgId` is checked by the rule above. Export
writes one root `PolicySet` per document with `PolicyCombiningAlgId`
`urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:deny-overrides`,
the `Description` and patient match the profile requires, and one `Policy`
per directive: a `permit` directive with `RuleCombiningAlgId`
`urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:deny-overrides`, a
`deny` directive with
`urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:permit-overrides`,
the base as one rule and each exception as one rule, so that an exception
decides where it applies. The document entry carries `typeCode` `57016-8`
and `formatCode` `urn:ihe:iti:appc:2016:consent` (5.6.2.2.1.1, .3).

### EHDS

| Right | openEHR form |
|---|---|
| Art. 8 restriction | `restriction` deny directive, `caller_kind` `professional`, with optional `data` scope and exceptions; invisible (step 2 and the read rule) |
| Art. 11(5) vital interests | `emergency_override` true on the restriction; the override is logged and shown to the patient |
| Art. 10 opt-out | `opt_out` deny directive, whole EHR, `caller_kind` `access_service`; reversible by a later version without it; `emergency_override` set by the Member State's rule (Art. 10(2), recital 55) |
| Art. 4(2) proxies | the representation and proxy `PARTY_RELATIONSHIP` of #5446, narrowed by a `delegation` directive |
| Art. 3(3) delay | stays the access-delay register (#5442); it is a controller rule on categories, not a patient's choice |
| Annex II 3.4 | `data.categories` and `data.origins` in directives and classifications |
| Annex II 3.2 | the access record with its grounds |

Today's restriction and opt-out registers (`clinical.access_restriction`,
`clinical.access_opt_out`) and their admin routes move into `EHR_ACCESS`
directives. That puts the patient's choices in the record they concern, makes
them travel with an extract, and keeps their history as versions.

## Cedar today, decision by decision

What the Cedar schema exposes (`app/ferroehr-rest/src/extensions/access/authz/request.rs`,
`model()`): the principal `User::"<sub>"` with `roles`, `scopes` and optional
`organization` and `patient` (from token claims); six resource types with
optional `patient`, `template`, `category` and `origin`; thirty actions
`<kind>.<mode>`; and the context `operation_id`. Policies are free boolean
expressions, `forbid` overrides `permit`, and multi-valued attributes are
decided over the cartesian product. The shipped examples
(`app/ferroehr-rest/examples/policies/consent.cedar`) are the first four rows.

| What a deployment writes in Cedar | The openEHR form | Kept? |
|---|---|---|
| Permit when the caller has an organisation, the patient matches, and the template is on an allow-list | Organisation: the caller's acts-for or employment relationship (5.7.1). Patient match: the subject gate and representation, already enforced before Cedar. Template allow-list: the `templates` scope of the caller's capability (5.7.1), or a classification plus clearance | Yes |
| Permit a break-glass role to reach any patient | `[authz.rbac] emergency_access_role` with an emergency purpose, and each directive's `emergency_override` | Yes, with the purpose required (stricter) |
| Forbid writes to a read-scoped client | The SMART scope layer and the 5.7.1 operation codes: a client without `commit_composition` cannot write | Yes |
| Forbid a revoked template everywhere | A classification giving the template a sensitivity nobody is cleared for, or removing it from the capabilities' template scope | Yes |
| Lab reports from origin X readable only by organisation Y's staff (the book's example) | Classification `{categories: lab, origins: X}` to `LAB-X`, and a clearance of `LAB-X` on the capabilities of Y's roles | Yes |
| Rule on one caller (`principal == User::"alice"`) | A directive or capability on Alice's `PERSON` | Yes |
| Rule on an operation id (`context.operation_id`) | The operation codes of the capability and of a directive's `operations` | Yes, at the grain of the terminology's codes |
| Rule on one resource kind and mode (`composition.read`) | The operation codes | Yes |
| Rule on a category or origin alone | Classification and clearance, or a directive's data scope | Yes |
| Rule on a raw OAuth scope string | The SMART scope layer decides scopes; ABAC no longer reads them | No |
| Rule on the raw organisation or patient claim string | Party data replaces the claims (the organisation claim goes; the patient claim stays an authentication binding) | No, by design |
| Arbitrary boolean composition (`A && (B \|\| !C)` across attributes) | Directives combine by deny-overrides with one level of exceptions; classifications combine by union | No |

**Cedar today cannot express** what this model adds: purpose of use, care
relationships, the patient's own directives, time validity of a grant finer
than a day, per-object sensitivity, and the EHDS invisibility of a
restriction. Its context holds only `operation_id`.

**The loss, and whether it matters for a CDR.** The three rows marked "No"
are free conditions over token strings and free boolean composition. A
deployment that needs them would be expressing policy outside the openEHR
model, which is what the owner's direction removes; the realistic cases are
covered by the scope layer, the operation codes and the break-glass floor.
The residual risk is a deployment with an unusual rule; its route is a new
code or attribute in the model, which then serves every deployment.

### The remote PDP

`[authz.abac] engine = "remote"` sends the same attributes to an external
decision point over HTTP per operation. It is the same second policy world,
adds an outbound call to every clinical request, and receives none of the
facts this design adds. National consent services (the Dutch Mitz among
them) are consulted by the federation gateway at the exchange, not by the
CDR. Recommendation: remove it in the same cut, as its own issue so the
owner rules on it separately.

## Own design

No specification governs these points; each is FerroEHR's own design, and
each is labelled so in `docs/architecture.md` when it is built:

- the scheme `access_directives` and every class of the draft package;
- facts in content and a fixed evaluator, with deny-overrides and one level
  of exceptions;
- the decision procedure, its order and its `refused-*` codes;
- care relationships through care-team `GROUP`s and `under care of`
  relationships, and the grace period;
- the sensitivity clearance and permitted purpose as capability codes;
- `ACCESS_POLICY` as a versioned root outside the EHR and the demographic
  service, and its extension resource;
- the read and write rule for `EHR_ACCESS`;
- the projection and its `LISTEN`/`NOTIFY` invalidation.

## Build plan

Greenfield (owner, 2026-11-12): no migration tool, conversion or retirement
aid. A removed key is an unknown key; a stored Cedar register version, the
`FERROEHR_ACCESS_CONTROL_V1` scheme and the restriction and opt-out
registers are dropped by migration. Every issue lists the configuration keys
it removes and adds. In order, each blocked by the one before unless noted:

1. **Vendor FHIR `Consent` (R4, R5), IHE BPPC and IHE APPC** under
   `docs/specs/` with provenance, and re-check every row of the
   [Mappings](#mappings) against them. AC: the texts are vendored with
   sha256 pins; the mapping table cites section by section; any row that
   does not hold is corrected here before issue 10. Done in #5505: the
   corrections are in the three mapping sections.
2. **Generate the draft types.** Depends on #5124 landing. Add the access
   control BMM as a second draft row; decide the re-emitted RM classes;
   hand-written invariant checks with one refusing test per invariant;
   round-trip tests of the canonical JSON. AC: as #5124's, for this package;
   `codegen-drift` covers it.
3. **The terminology groups.** Add the groups and codes to the FerroEHR
   terminology #5433 introduces, documented in the book as the stable
   contract. AC: the evaluator and the write checks read only these codes; a
   test refuses a code outside its group.
4. **The `access_directives` scheme on `EHR_ACCESS`.** Replace
   `FERROEHR_ACCESS_CONTROL_V1`: strict write check, the gate-keeper rule, the
   read rule (`404` to everyone else), the default archetype and template
   stored at boot. Removes nothing from TOML. AC: a test per invariant, per
   writer kind, and per reader kind; an old-scheme body is refused `400`.
5. **The projection and its cache.** `access_directive` and label side
   tables written in the commit's transaction and rebuilt by refill; the
   per-EHR cache invalidated by `LISTEN`/`NOTIFY`; the few-second expiry
   removed. AC: a revoked directive decides the next request on a second
   replica (test); a lost listener reads per request (test); the perf gate
   shows no added transaction.
6. **Restriction and opt-out as directives.** Move EHDS Art. 8 and Art. 10
   into the scheme; drop the two registers, their tables and admin routes;
   the controller's route commits an `EHR_ACCESS` version. AC: every test of
   today's restriction and opt-out behaviour passes against directives,
   including invisibility and the override marks.
7. **Caller facts.** Extend the 5.7.1 side table with care-team ids,
   clearances and permitted purposes; the `under care of` and `member of`
   relationships with their archetypes; the `write_care_relationship` grant.
   Blocked by #5435. AC: one indexed read returns them (plan test); an
   admission and a discharge decide the next request (test).
8. **`ACCESS_POLICY`.** Storage as a versioned object with contributions and
   audit, the extension resource (`GET`, `PUT` with `If-Match`, history), the
   default archetype, in-memory with invalidation. AC: as the registers of
   #5459, plus strict BMM checks.
9. **The evaluator.** The decision procedure, the refusal codes and the AQL
   condition. AC: a table-driven test per step and per refusal code, the
   worked examples above as end-to-end tests, an AQL test per step that
   counts and pages correctly, and the perf gate unchanged.
10. **Audit grounds.** The access record names the grounds; the `ehds-logging`
    record carries them. AC: a test per ground kind; refusals logged with
    their code, the invisible ones included.
11. **Remove Cedar.** Delete the Cedar engine, `RegisterCedar`, the
    `cedar-policies` register (its name, routes, OpenAPI, the stored rows by
    migration), `request::model()`, the example policies, the Cedar tests
    and the `cedar` dependency of the workspace. Remove `[authz.abac] enabled`,
    `engine` and `organization_claim`; `check_directory` becomes always on.
    Keep `patient_claim` and `patient_namespace` as authentication binding
    (moved under `[auth]`). AC: the keys are unknown keys; no Cedar remains
    in the tree; the book and the generated reference are updated; the
    release notes give the key count before and after.
12. **Remove the remote PDP** (owner ruling). Delete `[authz.abac.remote]`
    and `[authz.abac.policy]` and the `ferrohealth-authz` remote client use.
    AC: as issue 11.
13. **FHIR `Consent` and IHE APPC/BPPC import and export.** After 1 and 9.
    AC: every vendored `Consent` example imports, or is refused, as the
    FHIR section states; round trips of directives through each format;
    each listed loss refused with the element named.
14. **Docs.** `docs/architecture.md` records the design with its citations
    and own-design labels (the acceptance criterion of #5487); the book's
    security page loses the Cedar section and gains "Access directives",
    "Care relationships", "Sensitivity" and "The access policy";
    operational-policy and config-auth follow. AC: the pages describe only
    what ships.
15. **Proposal.** Write the specification pages and render the class
    partials from the BMM; nothing is sent to openEHR without the owner's
    approval.

Issues 4 to 10 land together in one release, so no release ships ABAC with
half its facts. Issue 11 lands in the same release as 9, so the release that
gains the evaluator loses Cedar.

## Proposal path

The package is written as an openEHR RM proposal: a concrete settings scheme
for the slot the RM leaves open, and a controller policy root. A Change
Request would cite RM EHR IM §EHR Access and BASE Security §Access Control,
which name the slot and ask for "alternative models of access control, each
defined by a subtype". The terminology groups would go to TERM. Nothing has
been submitted.

## Tasks

- [x] Read the governing texts and today's ABAC code.
- [x] The model, the BMM, the mappings, the Cedar inventory and the build
  plan.
- [x] The BMM composed, emitted and compiled with the repository's loader.
- [ ] The owner's decision.
- [ ] The build issues filed from the decision.

## Exit criteria

- [ ] The owner has decided: remove Cedar or keep it, and the remote PDP.
- [ ] The build issues are filed as sub-issues of #5487 with their
  acceptance criteria.
- [ ] This plan is deleted in the PR that lands issue 14.
