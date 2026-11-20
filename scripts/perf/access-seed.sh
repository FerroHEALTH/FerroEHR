#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The access content of the performance gate (#5590): what puts the access
# evaluator, its care relationships and controller rules, and the signed
# access log
# on every clinical request the plan sends. Sourced by scripts/perf/gate.sh,
# which defines `call`, `die`, SERVER_URL, BASE, AUTH and OUT; never run
# directly.
#
# An arm whose tree configures the clinician Basic user gets:
#   - the administrator's grants to write care relationships and the access
#     policy, through `ferroehr party-binding grant`;
#   - an ACCESS_POLICY that requires a care relationship with a 14-day grace,
#     classifies the lab results the read EHR holds as HIV results, and holds
#     a controller rule that lets the clinician's role read HIV data (#5621);
#   - a care team GROUP the clinician is `member_of`, and a ROLE the
#     clinician performs, named by that role's code;
#   - patients `under_care_of` that team, the subjects of the read EHR, the
#     write EHR and every population EHR;
#   - on each of those EHRs, settings of five directives: treatment consent
#     for the care relationship, a mental-health restriction excepting one
#     professional, an HIV restriction, a research opt-out and a
#     representative.
# CLINICAL_AUTH is then the clinician's credential. An arm whose tree has no
# clinician (an older base) gets none of it, and CLINICAL_AUTH is the
# administrator's. No openEHR spec governs this seed: our own design.

# The extension routes (the ADMIN group, the openEHR access log, the export).
readonly EXT=/ferroehr/rest/extensions/v1
# shellcheck disable=SC2034 # read by gate.sh, which sources this file
readonly CLINICIAN_AUTH="Basic Y2xpbmljaWFuOmNsaW5pY2lhbg=="
# The access-control codes and the grant operations (openehr_access). The
# terminology identifiers of a directive's effect (HL7 ConsentProvisionType)
# and of a care-team membership (HL7 RoleClass MBR) are read from the
# terminology of the tree the seeded server is built from (access_vocabulary),
# so a base built before a rename is seeded with the identifiers it accepts.
readonly ACCESS_TERMINOLOGY=openehr_access
PROVISION_TERMINOLOGY=""
ROLE_CLASS_TERMINOLOGY=""
# Set by access_vocabulary when the tree writes a care relationship as a
# PARTY_RELATIONSHIP of its own (#6055): the care_role terminology and the
# provider the clinician acts for, as "<issuer> <id>". Empty for a tree that
# holds it in the patient's relationships.
PARTICIPATION_TERMINOLOGY=""
CARE_PROVIDER=""
readonly ACT_CODE=2.16.840.1.113883.5.25
readonly ACT_REASON=2.16.840.1.113883.5.8
# The code system and code of the clinician's functional role.
readonly ROLE_SYSTEM=urn:ferroehr:perf:roles
readonly ROLE_CODE=INFECTIOUS_DISEASE
# What access_seed creates: the care team, the professional the mental-health
# restriction excepts and the subjects' representative (parties the
# directives name, never callers of the plan).
ACCESS_TEAM=""
NAMED_PROFESSIONAL=""
REPRESENTATIVE=""
# What gate.sh sets for the arm: the command that runs its binary against its
# database, and the file scim-provision.sh named each User it created in
# (SCIM_IDS), whose id is the container of the clinician's PERSON.
ACCESS_FERROEHR=()
ACCESS_IDS=""

# access_arm <tree>: whether the arm's tree runs the clinician.
access_arm() {
  grep -q '^user = "clinician"$' "$1/perf/ferroehr.toml"
}

# A DV_CODED_TEXT of terminology $1 and code $2.
coded_json() {
  jq -cn --arg t "$1" --arg c "$2" \
    '{_type: "DV_CODED_TEXT", value: $c, defining_code: {_type: "CODE_PHRASE",
      terminology_id: {_type: "TERMINOLOGY_ID", value: $t}, code_string: $c}}'
}

# party_json <rm> <archetype> <name> [<relationships-json>]: a party body.
party_json() {
  jq -cn --arg rm "$1" --arg a "$2" --arg n "$3" --argjson rel "${4:-[]}" '
    def text($v): {_type: "DV_TEXT", value: $v};
    {_type: $rm, archetype_node_id: $a,
     archetype_details: {_type: "ARCHETYPED", rm_version: "1.1.0",
       archetype_id: {_type: "ARCHETYPE_ID", value: $a}},
     name: text($n),
     identities: [{_type: "PARTY_IDENTITY", archetype_node_id: "at0001", name: text("name"),
       details: {_type: "ITEM_TREE", archetype_node_id: "at0002", name: text("name"),
         items: [{_type: "ELEMENT", archetype_node_id: "at0003", name: text("name"),
           value: text($n)}]}}]}
    + (if ($rel | length) > 0 then {relationships: $rel} else {} end)'
}

# care_json <under_care_of|member_of> <source-person> <team>: a care
# relationship from the first of January this year, unbounded, coded
# under_care_of of openehr_access or MBR of the HL7 v3 RoleClass.
care_json() {
  local from t="$ACCESS_TERMINOLOGY" coded="$1"
  from="$(date -u +%Y)-01-01"
  if [[ "$1" == member_of ]]; then
    t="$ROLE_CLASS_TERMINOLOGY"
    coded=MBR
  fi
  jq -cn --arg code "$1" --arg coded "$coded" --arg source "$2" --arg team "$3" \
    --arg from "$from" --arg t "$t" '
    def text($v): {_type: "DV_TEXT", value: $v};
    def ref($type; $id): {_type: "PARTY_REF", namespace: "demographic", type: $type,
      id: {_type: "HIER_OBJECT_ID", value: $id}};
    {_type: "PARTY_RELATIONSHIP",
     archetype_node_id: ("openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP." + $code + ".v1"),
     name: text($code),
     details: {_type: "ITEM_TREE", archetype_node_id: "at0002", name: text("details"),
       items: [{_type: "ELEMENT", archetype_node_id: "at0003", name: text("kind"),
         value: {_type: "DV_CODED_TEXT", value: $code, defining_code: {_type: "CODE_PHRASE",
           terminology_id: {_type: "TERMINOLOGY_ID", value: $t}, code_string: $coded}}}]},
     time_validity: {_type: "DV_INTERVAL", lower: {_type: "DV_DATE", value: $from},
       lower_unbounded: false, upper_unbounded: true, lower_included: true,
       upper_included: false},
     source: ref("PERSON"; $source), target: ref("GROUP"; $team)}'
}

# care_of_json <patient> <team>: a care relationship of its own from the
# first of January this year, unbounded, from the patient's PERSON to the
# team's GROUP, in the role RESP under CARE_PROVIDER (draft access control
# package, Care Relationships).
care_of_json() {
  local from issuer id
  from="$(date -u +%Y)-01-01"
  read -r issuer id <<<"$CARE_PROVIDER"
  jq -cn --arg source "$1" --arg team "$2" --arg from "$from" --arg t "$ACCESS_TERMINOLOGY" \
    --arg roles "$PARTICIPATION_TERMINOLOGY" --arg issuer "$issuer" --arg id "$id" '
    def text($v): {_type: "DV_TEXT", value: $v};
    def code($term; $c): {_type: "DV_CODED_TEXT", value: $c, defining_code: {_type: "CODE_PHRASE",
      terminology_id: {_type: "TERMINOLOGY_ID", value: $term}, code_string: $c}};
    def ref($type; $id): {_type: "PARTY_REF", namespace: "demographic", type: $type,
      id: {_type: "HIER_OBJECT_ID", value: $id}};
    {_type: "PARTY_RELATIONSHIP",
     archetype_node_id: "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v2",
     name: code($t; "under_care_of"),
     details: {_type: "ITEM_TREE", archetype_node_id: "at0001", name: text("details"),
       items: [
         {_type: "ELEMENT", archetype_node_id: "at0002", name: code($t; "care_role"),
          value: code($roles; "RESP")},
         {_type: "ELEMENT", archetype_node_id: "at0003", name: code($t; "provider"),
          value: {_type: "DV_IDENTIFIER", issuer: $issuer, id: $id, type: "ORGANISATION"}}]},
     time_validity: {_type: "DV_INTERVAL", lower: {_type: "DV_DATE", value: $from},
       lower_unbounded: false, upper_unbounded: true, lower_included: true,
       upper_included: false},
     source: ref("PERSON"; $source), target: ref("GROUP"; $team)}'
}

# The clinician's ROLE, named by the role's code, granting access_demographic.
clinician_role_json() {
  jq -cn --arg performer "$1" --argjson name "$(coded_json "$ROLE_SYSTEM" "$ROLE_CODE")" \
    --argjson op "$(coded_json "$ACCESS_TERMINOLOGY" access_demographic)" '
    def text($v): {_type: "DV_TEXT", value: $v};
    {_type: "ROLE", archetype_node_id: "openEHR-DEMOGRAPHIC-ROLE.ferroehr_grant.v1",
     archetype_details: {_type: "ARCHETYPED", rm_version: "1.2.0",
       archetype_id: {_type: "ARCHETYPE_ID", value: "openEHR-DEMOGRAPHIC-ROLE.ferroehr_grant.v1"}},
     name: $name,
     identities: [{_type: "PARTY_IDENTITY", archetype_node_id: "at0001", name: text("name"),
       details: {_type: "ITEM_TREE", archetype_node_id: "at0002", name: text("details"),
         items: [{_type: "ELEMENT", archetype_node_id: "at0003", name: text("name"),
           value: text("access_demographic")}]}}],
     performer: {_type: "PARTY_REF", namespace: "demographic", type: "PERSON",
       id: {_type: "HIER_OBJECT_ID", value: $performer}},
     capabilities: [{_type: "CAPABILITY", name: text("grant"), archetype_node_id: "at0020",
       credentials: {_type: "ITEM_TREE", name: text("terms"), archetype_node_id: "at0023",
         items: [{_type: "ELEMENT", name: text("operation"), archetype_node_id: "at0021",
           value: $op}]}}]}'
}

# The ACCESS_POLICY: a care relationship with a 14-day grace, the lab results
# classified as HIV results, and a controller rule letting the clinician's
# role read HIV data.
policy_json() {
  jq -cn --arg controller "$1" --argjson hiv "$(coded_json "$ACT_CODE" HIV)" \
    --argjson role "$(coded_json "$ROLE_SYSTEM" "$ROLE_CODE")" \
    --argjson rule "$(coded_json "$ACCESS_TERMINOLOGY" controller_rule)" \
    --argjson permit "$(coded_json "$PROVISION_TERMINOLOGY" permit)" '
    def text($v): {_type: "DV_TEXT", value: $v};
    {_type: "ACCESS_POLICY", name: text("Perf gate access policy"),
     archetype_node_id: "openEHR-EHR-ACCESS_POLICY.controller.v1",
     archetype_details: {_type: "ARCHETYPED", rm_version: "1.1.0",
       archetype_id: {_type: "ARCHETYPE_ID", value: "openEHR-EHR-ACCESS_POLICY.controller.v1"}},
     controller: {_type: "PARTY_REF", namespace: "demographic", type: "ORGANISATION",
       id: {_type: "HIER_OBJECT_ID", value: $controller}},
     care_relationship_required: true,
     relationship_grace: {_type: "DV_DURATION", value: "P14D"},
     purpose_required: false,
     classifications: [{_type: "ACCESS_CLASSIFICATION", name: text("HIV results"),
       archetype_node_id: "at0002",
       data: {_type: "ACCESS_DATA_SCOPE", archetypes: [{_type: "ARCHETYPE_ID",
         value: "openEHR-EHR-OBSERVATION.laboratory_test_result.v1"}]},
       sensitivities: [$hiv]}],
     rules: [{_type: "ACCESS_DIRECTIVE", name: text("The role reads HIV data"),
       archetype_node_id: "at0003", kind: $rule, effect: $permit,
       applies_to: [{_type: "ACCESS_ACTOR_SCOPE", role: $role}],
       data: [{_type: "ACCESS_DATA_SCOPE", sensitivities: [$hiv]}],
       emergency_override: false}]}'
}

# The settings every seeded EHR carries: five directives, one gate-keeper.
settings_json() {
  jq -cn --arg t "$ACCESS_TERMINOLOGY" --arg provision "$PROVISION_TERMINOLOGY" \
    --arg act "$ACT_CODE" --arg reason "$ACT_REASON" \
    --arg named "$NAMED_PROFESSIONAL" --arg rep "$REPRESENTATIVE" '
    def text($v): {_type: "DV_TEXT", value: $v};
    def code($term; $c): {_type: "DV_CODED_TEXT", value: $c, defining_code: {_type: "CODE_PHRASE",
      terminology_id: {_type: "TERMINOLOGY_ID", value: $term}, code_string: $c}};
    def term($c): code($t; $c);
    def person($id): {_type: "PARTY_REF", namespace: "demographic", type: "PERSON",
      id: {_type: "HIER_OBJECT_ID", value: $id}};
    def directive($kind; $effect): {_type: "ACCESS_DIRECTIVE", name: text($kind),
      archetype_node_id: "at0001", kind: term($kind), effect: code($provision; $effect),
      emergency_override: true};
    {_type: "ACCESS_DIRECTIVE_SETTINGS", default_access: term("open"),
     gate_keepers: [{_type: "PARTY_IDENTIFIED", external_ref: person($named)}],
     directives: [
       directive("consent"; "permit") + {
         applies_to: [{_type: "ACCESS_ACTOR_SCOPE", relationship: term("care")}],
         operations: [term("read"), term("query")], purposes: [code($reason; "TREAT")]},
       directive("restriction"; "deny") + {
         data: [{_type: "ACCESS_DATA_SCOPE",
           categories: [code("urn:example:categories"; "mental-health")]}],
         exceptions: [directive("consent"; "permit") + {
           applies_to: [{_type: "ACCESS_ACTOR_SCOPE", party: person($named)}]}]},
       directive("restriction"; "deny") + {
         data: [{_type: "ACCESS_DATA_SCOPE", sensitivities: [code($act; "HIV")]}]},
       directive("opt_out"; "deny") + {
         operations: [term("export"), term("query")], purposes: [code($reason; "HRESCH")]},
       directive("delegation"; "permit") + {
         applies_to: [{_type: "ACCESS_ACTOR_SCOPE", relationship: term("representative"),
           party: person($rep)}]}]}'
}

# The EHR_STATUS of an EHR whose subject is the PERSON $1, named in this
# CDR's own demographic namespace, whose references name a PERSON by its
# container ([authz.party_binding] namespace, by default `demographic`).
subject_status_json() {
  jq -cn --arg id "$1" '
    {_type: "EHR_STATUS", archetype_node_id: "openEHR-EHR-EHR_STATUS.generic.v1",
     archetype_details: {_type: "ARCHETYPED", rm_version: "1.1.0",
       archetype_id: {_type: "ARCHETYPE_ID", value: "openEHR-EHR-EHR_STATUS.generic.v1"}},
     name: {_type: "DV_TEXT", value: "EHR status"},
     subject: {_type: "PARTY_SELF", external_ref: {_type: "PARTY_REF", namespace: "demographic",
       type: "PERSON", id: {_type: "HIER_OBJECT_ID", value: $id}}},
     is_queryable: true, is_modifiable: true}'
}

# post_json <path> <json>: the response body of a POST of <json>, wanting 201.
post_json() {
  local body="$OUT/access-seed.$BASHPID.json"
  printf '%s' "$2" >"$body"
  call POST "$1" application/json "$body" 201
  rm -f "$body"
}

# put_json <path> <preceding-version> <json>: a PUT of <json> over
# <preceding-version>, wanting 200.
put_json() {
  local body="$OUT/access-seed.$BASHPID.json" answer="$OUT/access-seed.$BASHPID.answer" status
  printf '%s' "$3" >"$body"
  status=$(curl -sS -o "$answer" -w '%{http_code}' -X PUT -H "authorization: $AUTH" \
    -H "accept: application/json" -H "prefer: return=minimal" -H "if-match: $2" \
    -H "content-type: application/json" --data-binary "@$body" "$SERVER_URL$BASE$1")
  [[ "$status" == 200 || "$status" == 204 ]] \
    || die "seed: PUT $1 answered $status, want 200: $(head -c 400 "$answer")"
  rm -f "$body" "$answer"
}

# related <kind> <archetype> <name> <code> <team> [<person-id> <version>]: a
# party of <kind> holding a care relationship of <code> to <team>, created
# first when no <person-id> is given, since a relationship is stored under its
# source and a new party's id is assigned by its commit; its id on stdout.
related() {
  local kind="$1" archetype="$2" name="$3" code="$4" team="$5" id="${6:-}" version="${7:-}"
  if [[ -z "$id" ]]; then
    version=$(post_json "/demographic/$kind" "$(party_json PERSON "$archetype" "$name")" \
      | jq -er '.uid.value')
    id="${version%%::*}"
  fi
  put_json "/demographic/$kind/$id" "$version" \
    "$(party_json PERSON "$archetype" "$name" "[$(care_json "$code" "$id" "$team")]")"
  echo "$id"
}

# access_patient_ehr <team>: one patient under the care of <team> and their
# EHR with the directives; the EHR id on stdout.
access_patient_ehr() {
  local patient ehr
  if [[ -n "$CARE_PROVIDER" ]]; then
    patient=$(post_json /demographic/person \
      "$(party_json PERSON openEHR-DEMOGRAPHIC-PERSON.person.v1 Patient)" \
      | jq -er '.uid.value | split("::")[0]')
    post_json "$EXT/demographic/party_relationship" "$(care_of_json "$patient" "$1")" >/dev/null
  else
    patient=$(related person openEHR-DEMOGRAPHIC-PERSON.person.v1 Patient under_care_of "$1")
  fi
  ehr=$(post_json /ehr "$(subject_status_json "$patient")" | jq -er '.ehr_id.value')
  access_directives "$ehr"
  echo "$ehr"
}

# access_directives <ehr>: the settings committed on <ehr>.
access_directives() {
  local version body="$OUT/access-settings.$BASHPID.json"
  version=$(call GET "$EXT/admin/ehr-access?ehr_id=$1" "" - 200 | jq -r '.versions[0].version_uid // empty')
  jq -cn --arg ehr "$1" --arg v "$version" --argjson s "$(settings_json)" \
    '{ehr_id: $ehr, preceding_version_uid: (if $v == "" then null else $v end), settings: $s}' \
    >"$body"
  call POST "$EXT/admin/ehr-access" application/json "$body" 200 >/dev/null
  rm -f "$body"
}

# access_population <count>: <count> patients under the care of the team, each
# with an EHR carrying the directives, eight at a time.
access_population() {
  local count="$1" i made="$OUT/access-population.ids"
  : >"$made"
  for ((i = 0; i < count; i++)); do
    access_patient_ehr "$ACCESS_TEAM" >>"$made" 2>>"$OUT/access-population.log" &
    if (($(jobs -pr | wc -l) >= 8)); then wait -n || true; fi
  done
  wait || true
  [[ "$(wc -l <"$made" | tr -d ' ')" == "$count" ]] \
    || die "seed: $(wc -l <"$made" | tr -d ' ') of $count access population EHRs were made: $(head -c 400 "$OUT/access-population.log")"
}

# access_vocabulary <tree>: the terminology identifiers of a directive's
# effect and of a care-team membership, as <tree>'s openehr-rm names them.
access_vocabulary() {
  local terms="$1/crates/openehr-rm/src/draft/access_control/terminology.rs"
  PROVISION_TERMINOLOGY=$(sed -nE 's/^pub const TERMINOLOGY_ID_CONSENT_PROVISION_TYPE: &str = "([^"]+)";$/\1/p' "$terms")
  ROLE_CLASS_TERMINOLOGY=$(sed -nE 's/^pub const TERMINOLOGY_ID_ROLE_CLASS: &str = "([^"]+)";$/\1/p' "$terms")
  if [[ -z "$PROVISION_TERMINOLOGY" || -z "$ROLE_CLASS_TERMINOLOGY" ]]; then
    echo "access seed: $terms names no effect or membership terminology" >&2
    return 1
  fi
  PARTICIPATION_TERMINOLOGY=""
  CARE_PROVIDER=""
  if [[ -f "$1/app/ferroehr/src/party_binding/grants/caller/care_of.rs" ]]; then
    PARTICIPATION_TERMINOLOGY=$(sed -nE 's/^pub const TERMINOLOGY_ID_PARTICIPATION_TYPE: &str = "([^"]+)";$/\1/p' "$terms")
    CARE_PROVIDER=$(sed -nE 's/^provider = \{ issuer = "([^"]+)", id = "([^"]+)" \}$/\1 \2/p' "$1/perf/ferroehr.toml")
    if [[ -z "$PARTICIPATION_TERMINOLOGY" || -z "$CARE_PROVIDER" ]]; then
      echo "access seed: $1 names no care_role terminology or no provider in perf/ferroehr.toml" >&2
      return 1
    fi
  fi
}

# access_seed [<tree>]: the grants, the policy, the team, the clinician's
# membership and role, and the parties the directives name, through
# ACCESS_FERROEHR and ACCESS_IDS, in the vocabulary of <tree> (default ROOT).
access_seed() {
  local op clinician version org
  access_vocabulary "${1:-$ROOT}"
  for op in write_care_relationship write_access_policy; do
    "${ACCESS_FERROEHR[@]}" party-binding grant --basic-user ferroehr --operation "$op" >/dev/null
  done
  NAMED_PROFESSIONAL=$(post_json /demographic/person \
    "$(party_json PERSON openEHR-DEMOGRAPHIC-PERSON.person.v1 "Named professional")" \
    | jq -er '.uid.value | split("::")[0]')
  REPRESENTATIVE=$(post_json /demographic/person \
    "$(party_json PERSON openEHR-DEMOGRAPHIC-PERSON.person.v1 Representative)" \
    | jq -er '.uid.value | split("::")[0]')
  org=$(post_json /demographic/organisation \
    "$(party_json ORGANISATION openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1 Hospital)" \
    | jq -er '.uid.value | split("::")[0]')
  printf '%s' "$(policy_json "$org")" >"$OUT/access-policy.json"
  curl -fsS -o /dev/null -X PUT -H "authorization: $AUTH" -H "content-type: application/json" \
    -H "if-none-match: *" --data-binary "@$OUT/access-policy.json" \
    "$SERVER_URL$EXT/admin/access_policy" || die "seed: the access policy was refused"
  ACCESS_TEAM=$(post_json /demographic/group \
    "$(party_json GROUP openEHR-DEMOGRAPHIC-GROUP.care_team.v1 Cardiology)" \
    | jq -er '.uid.value | split("::")[0]')
  clinician=$(awk '$1 == "clinician" { print $2 }' "$ACCESS_IDS")
  [[ -n "$clinician" ]] || die "seed: SCIM created no clinician"
  # The PERSON SCIM wrote, kept whole, with the membership added.
  call GET "/demographic/person/$clinician" "" - 200 >"$OUT/access-clinician.json"
  version=$(jq -er '.uid.value' "$OUT/access-clinician.json")
  put_json "/demographic/person/$clinician" "$version" \
    "$(jq -c --argjson care "$(care_json member_of "$clinician" "$ACCESS_TEAM")" \
      'del(.uid) | .relationships = ((.relationships // []) + [$care])' "$OUT/access-clinician.json")"
  post_json /demographic/role "$(clinician_role_json "$clinician")" >/dev/null
}
