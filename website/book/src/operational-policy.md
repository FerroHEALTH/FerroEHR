# Operational policy

Policy that changes while the CDR runs is data the CDR holds, never a key in
`ferroehr.toml`: a hospital CDR cannot be stopped to edit a file. Each change
is versioned, carries who made it, is written to the access log, and decides
the next request on every replica without a restart. `ferroehr.toml` keeps
only what the server needs before it can reach its database or trust a
caller, such as the listeners, the issuers, `admin_role`, the deployment
profile and the security floors.

No openEHR specification governs operational policy or where it is kept; the
design on this page is FerroEHR's own.

<!-- toc -->

## The admin registers

Three kinds of operational policy have no home in the openEHR demographic
model, so each is an admin register:

| Register | What it holds | Default, before anyone writes it |
|---|---|---|
| `audit-categories` | The EHDS category map every access is classified through ([the audit category register](installation/config-audit.md#the-audit-category-register)) | The category map FerroEHR ships |
| `access-delay` | The EHDS Art. 3(3) delay of the person's own access ([the access-delay register](installation/config-auth.md#the-access-delay-register)) | Off |
| `cedar-policies` | The policy set of the embedded Cedar engine ([below](#the-cedar-policy-register)) | Empty, so Cedar permits nothing |

You read and write them under `{ext}/admin/registers/{register}` with a token
or Basic user holding `admin_role`, with `[api] admin = true`; the routes are
on [Admin & messaging APIs](operations-admin-apis.md#operational-policy-registers).

- **Versioned.** Every write appends a version that names its committer and
  when it was committed, and never overwrites one; the history route lists
  them all. Version 0 is the default no one committed.
- **Whole, over a version.** A write replaces the whole document and names the
  version it replaces in `If-Match`. A write over a version that is no longer
  the newest is refused with `412`, so two operators never overwrite each
  other's change unseen.
- **Checked when written.** A document that does not hold is refused whole with
  `400` and never reaches a request: a category map that does not compile, a
  delay with an unusable limit, a Cedar set that does not parse or validate.
  The register keeps its version in force.
- **Effective on the next request.** Every request that a register can
  decide reads the newest version of each register first, one indexed read, so a change applies to the next
  request on every replica. A database that cannot answer that read fails the
  request with `503`; it is never served under a guess.
- **Audited.** Every read and write of a register is an access record.

The three keys these registers replace, `[audit.categories]`,
`[privacy.access_delay]` and `[authz.abac.cedar]`, are unknown keys in
`ferroehr.toml` and refuse boot. A deployment provides its registers once the
server runs, through the admin API.

### A worked example: turning the access delay on

Read the register, then write the new document over the version it answered:

```sh
curl -s -u admin:… https://cdr.example/ferroehr/rest/extensions/v1/admin/registers/access-delay
# ETag: "0"
# {"register":"access-delay","version":0,"committer":null,"committed_at":null,
#  "document":{"enabled":false,"categories":[],"max_delay":null}}

curl -s -u admin:… -X PUT \
  -H 'Content-Type: application/json' -H 'If-Match: "0"' \
  -d '{"enabled": true, "categories": ["Laboratory-Reports"], "max_delay": "72h"}' \
  https://cdr.example/ferroehr/rest/extensions/v1/admin/registers/access-delay
# ETag: "1"
```

The next laboratory report committed on any replica is held from the person
for up to 72 hours.

### The Cedar policy register

The embedded Cedar engine (`[authz.abac] engine = "cedar"`) decides with the
policy set of this register, in the Cedar policy language:

```json
{ "policies": "permit(principal, action, resource) when { principal has organization };" }
```

A write is parsed and validated strictly against the schema FerroEHR builds
from its ABAC request model, and a set that does not parse or validate is
refused with `400`; the set in force keeps deciding. Until the register is
written the set is empty, and Cedar, deny-by-default, permits nothing, so a
deployment that turns ABAC on writes its policies before it serves clinical
traffic. The example policies under
[`app/ferroehr-rest/examples/policies`](https://github.com/FerroHEALTH/FerroEHR/tree/main/app/ferroehr-rest/examples/policies)
are a starting point.

### The security floor that stays in `ferroehr.toml`

A floor that must hold before a register can be read stays configuration. With
`deployment.profile = "production"` and auditing on, the audit category
register refuses a map that classifies nothing (no template, no archetype and
`shipped_map = false`) as the `no_category_map` gap, unless
`deployment.accepts` names it.
