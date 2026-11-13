# Breaking changes of party binding

The release that binds every person to a demographic `PERSON` changes the
configuration and the wire in ways an existing deployment must act on.
Each change below has the upgrade step it needs and its own entry under
Changed or Removed in the changelog. The order to apply them in is on
[Party binding: bootstrap and migration](party-binding-migration.md).

## Configuration

- **The family pins move together.** `ferrohealth-smart` 0.9, with
  `ferrohealth-authz`, `ferrohealth-session`, `ihe-atna`, `ehds-logging`,
  `ihe-iti`, `ferrohealth-config`, `ferrohealth-server` and
  `ferrohealth-ui`. Upgrade: nothing to set; the configuration changes
  below follow from the `ferrohealth-smart` 0.9 shape.
- **Every issuer is an `[[auth.issuer]]` table with a required `kind`**
  (`persons`, `patients`, `machines` or `federation`) and a profile, and
  holds every setting of its tokens itself. Upgrade: write each issuer as
  its own table, move the section-level settings (`audience`, `algorithms`,
  `clock_skew_seconds`, the `jwks_*` keys, `timeout_ms`, `mode`,
  `role_claims`, `[auth.introspection]`) onto it, give it its `kind`, and
  put patients and professionals at separate issuers.
- **`token_profile`, the section-level issuer settings and the `claim` key
  of `[[auth.party]]` are deleted.** A configuration that still names them
  refuses the start, naming what replaces it. Upgrade: replace
  `token_profile = "jwt"` with `token_types = ["JWT"]` and the
  `required_claims` the issuer's tokens carry, drop
  `token_profile = "rfc9068"`, and let the issuer profile's `person_key`
  name the key claim.
- **`acting_for_claim` and `client_tokens` are removed** with
  `[authz.professional]`. The issuer's `kind` and profile classify the
  caller. Upgrade: remove the table and list a client that acts for a
  person in its issuer's `delegates`.
- **Basic users are persons only.** Every `[[auth.basic.users]]` entry maps
  to a `PERSON` by its binding key in `[[authz.party_binding.basic_user]]`,
  an unmapped one refuses the start naming the user, and every machine,
  read-only included, uses OAuth 2.0 client credentials under a declared
  `[[authz.system_client]]`. Upgrade: provision each person a Basic user
  stands for through SCIM, map the user by that person's key, and move
  every machine to client credentials.
- **Every committing system client needs its declaration**, and the
  facility rule only checks. A system caller commits into an EHR only as a
  client an `[[authz.system_client]]` table declares, each COMPOSITION of a
  template its `writes` lists and naming a facility its `facilities`
  lists; the server never writes the facility for it. Upgrade: declare
  every client that commits, with its `writes`, `facilities` and
  `origins`, and give its compositions the facility identifier. From
  5.7.1 the table is gone and a system client is granted in the CDR
  ([Operational policy](../operational-policy.md#what-a-system-client-may-do)).
- **Import needs a role.** Importing an EHR Extract needs
  `[authz.rbac] import_role` or a `system/` caller. Upgrade: set
  `import_role` and give it to the accounts that import. From 5.7.1 a
  system caller imports only under an `import_ehr` or `import_ehr_extract`
  grant in the CDR.

## Binding and the committer

- **Unbound, inactive, abandoned and deprovisioned persons are refused.**
  A `persons` login whose key no `PERSON` carries is refused with `403`
  (`refused-unbound`), and so is one whose `PERSON` is `800|inactive|`,
  `801|abandoned|` or deleted. Upgrade: provision every professional before
  you add a `persons` issuer, through the four steps.
- **The professional `ROLE` is what makes a `PERSON` bindable.** A
  `PERSON` binds only while it performs a `ROLE` of the mapping's role
  template naming it as `performer`. Upgrade: give each existing person its
  `ROLE` with `ferroehr party-binding adopt` before the first SCIM run.
- **The committer is the server's.** A bound caller commits under the
  committer the server writes, and a stated committer must equal it or be
  the placeholder `{"_type": "PARTY_IDENTIFIED", "identifiers":
  [{"_type": "DV_IDENTIFIER", "issuer": "<iss>", "id": "<sub>"}]}`, which
  names the authenticated caller; any other is refused with `403`. Upgrade:
  stop sending a committer, or send the placeholder.
- **Every person's committer carries the provider identifier**, type
  `healthcare-provider`, beside the login identifier and the `external_ref`
  to the `PERSON`, and a representative commits as a `PARTY_RELATED`
  naming their own `PERSON` and the coded relationship. Upgrade: read the
  provider and the representative from these committers.
- **A patient's committer is `PARTY_SELF` without `external_ref`.**
  Upgrade: find a person's own entries by the committer's `PARTY_SELF`
  type, not by a former `external_ref`.
- **System callers and patient context are strict.** A system caller holds
  `system/` scopes only, and a token carrying a `user/` or `patient/` scope
  is refused with `403`; a patient-context token commits no demographic
  version. Upgrade: give each system client `system/` scopes alone.
- **The parties binding relies on are protected.** A key-bearing `PERSON`,
  its professional `ROLE`, a representation relationship and a provisioned
  `GROUP` are written through the demographic API by an administrator
  only, and a present binding key changes only through
  `POST /admin/party/binding-key`. Upgrade: write these parties as an
  administrator.

Versions committed before the upgrade keep their committer: versions are
immutable, and nothing rewrites an `AUDIT_DETAILS` (RM Common
`master06-change_control_package.adoc` §Committal and Audits).
