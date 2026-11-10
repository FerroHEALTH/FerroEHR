# Party binding: bootstrap and migration

FerroEHR binds every person who logs in to a demographic `PERSON`, and a
login with no `PERSON` is refused (`403`, `refused-unbound`). This page
takes a new deployment from an empty database to its first bound login, and
takes an existing deployment, whose people were attributed by login alone,
to binding. The configuration it names is in
[Authentication & access](config-auth.md#the-party-mapping), and the two
commands are in [CLI & production checklist](config-cli.md). Every
incompatible change of the release, with its upgrade step, is on
[Breaking changes of party binding](party-binding-breaking-changes.md).

No openEHR specification governs a deployment's migration or its
credential store: this is FerroEHR's own design.

## Who is a person

Every `[[auth.basic.users]]` entry is a person. It maps to the `PERSON`
that carries its binding key, with `[[authz.party_binding.basic_user]]`:

```toml
[[authz.party_binding.basic_user]]
user = "clinician"
key_issuer = "urn:example:staff-login"   # the identifier_issuer of an [[auth.party]] table
key = "clinician"                        # the binding key the PERSON carries
```

A Basic user with no mapping refuses the start, naming the user. A machine,
read-only included, authenticates with OAuth 2.0 client credentials under
a declared `[[authz.system_client]]`. A Basic credential carries no
assurance level, so with `[authz.assurance] minimum` set it is refused on
patient data, and the server logs a `warn` naming every mapped Basic user
while no minimum is set.

## Bootstrapping a new deployment

With Basic users persons only, a new deployment has no machine login to
start from, so it starts in this order:

1. **Declare the identity provider's SCIM client.** It is an OAuth 2.0
   client-credentials client (RFC 6749 §4.4) of the deployment's
   authorization server with the provisioning scope: an `[[auth.issuer]]`
   of kind `machines` that issues its tokens, and a `[[scim.client]]`
   naming that issuer, the client, the scope and the `binding_issuer` its
   Users' keys are written under.
2. **Provision the first person through `/scim/v2/`.** The identity
   provider, or you through that client, sends `POST /scim/v2/Users`
   (RFC 7644 §3.3). FerroEHR creates the `PERSON` and its professional
   `ROLE`, and issues the `PERSON`'s id itself (RFC 7643 §3.1: the `id` "is
   always issued by the service provider").
3. **Only then map a Basic user**, by that person's binding key. A Basic
   user mapped before its `PERSON` exists is refused at its first request.

The first administrator is a provisioned person whose token carries the
administrator role from an issuer `[authz.rbac] role_issuers` lists.

The [Docker Compose quickstart](compose.md) follows this path from an
empty database: its `ferroehr-provision` one-shot takes a token for the
`quickstart-scim` client and provisions the user `ferroehr` through
`/scim/v2/` once the server is healthy, and `ferroehr` is mapped by its
key. The hosted sandbox does the same with its `ferroehr-provision`
service after every nightly reset, and its reseed waits for the login to
bind before it commits the demo data.

## Migrating an existing deployment

An existing deployment attributes its people by login alone. It moves to
binding in four steps, each with its check. Add the `persons` issuers last:
from then on, every login with no `PERSON` is refused.

### 1. Prepare the existing persons

Bring every existing `PERSON`, for example those migrated from an earlier
system, to a template of the party mapping, with the login identifier at
the binding-key selector and the identity provider's `userName` at its
selector. Leave `externalId` to the identity provider, which issues it
(RFC 7643 §3.1). A prepared `PERSON` performs no professional `ROLE` yet,
so it does not bind.

**Adopt them.** `ferroehr party-binding adopt` gives each listed `PERSON`
a professional `ROLE` of the mapping's role template, with the `PERSON` as
`performer`, and a new `PERSON` version listing it in `roles`, in one
demographic CONTRIBUTION committed by the administrator you name, and
writes one access record per adopted `PERSON`:

```sh
ferroehr party-binding adopt --admin <ADMIN_PARTY_ID> \
  --migration "<committer name of the migration>" <PARTY_ID>...
```

It refuses, naming them, every `PERSON` whose binding key was introduced
by a committer the provenance rule does not accept (step 3), so a key a
demographic writer planted before the upgrade is never adopted silently.
Adopt before the first SCIM run: without the `ROLE`, the provider's
`userName eq` query finds no User and creates a duplicate.

At its next start, the server fills the binding side table from the party
versions and refuses to start on any duplicate key, naming the parties.

### 2. Upgrade, then provision through SCIM

Start the upgraded deployment with only the SCIM client's issuer
configured; the `persons` issuers come in step 4, so no clinician meets a
refusal while provisioning runs (for Microsoft Entra ID, the initial
provisioning cycle). The provider's `filter=userName eq "..."` queries find
the existing persons whose keys are equal, so it updates them, writing
their `externalId`, and creates only the professionals who have no
`PERSON` yet. A bulk load through SCIM is the one sanctioned route.

### 3. Run the pre-flight

```sh
ferroehr party-binding check --admin <ADMIN_PARTY_ID> \
  --migration "<committer name of the migration>" \
  --expected assigned-users.txt
```

`ferroehr party-binding check` reads the party versions and writes
nothing, so it runs before any boot fill. Per key issuer it reports the
persons that carry a key, duplicates, inactive and abandoned persons, the
provisional persons older than `provisional_report_after`, the key-bearing
persons that perform no professional `ROLE` (which `adopt` has not
reached), and every mapped Basic user with the `PERSON` it binds.

For each key it reports the committer of the version that introduced the
key, and fails unless that committer was a `[[scim.client]]`, an
administrator you name, a migration you declare, or just-in-time creation
on an issuer with `jit = true`. A stored version records no role its
committer held, so you name the administrators (`--admin`, `--admin-key`),
and the report repeats them.

With `--expected`, it reads the identity provider's list of assigned users
(their login identifiers, exported from the provider, one per line) and
lists every one with no `PERSON`. The command exits non-zero on a
duplicate, a missing person or a key that fails the provenance rule.

### 4. Add the `persons` issuers

Add each `persons` issuer with its `[[auth.party]]` table. Every login with
no `PERSON` is refused from then on; the `warn` lines and the
`binding_refusals` metric show any the pre-flight missed.

### Versions committed before binding

Versions are immutable, and nothing rewrites an `AUDIT_DETAILS` (RM Common
`master06-change_control_package.adoc` §Committal and Audits), so every
version committed before the upgrade keeps the committer it was committed
with. Only versions committed after it name the bound `PERSON`.

## Go-live prerequisites in other products

A deployment whose FerroBRIDGE facade or FerroFED gateway writes to
FerroEHR adds its `persons` issuers only after the bridge and the gateway
reach it under the caller; until then their writes are refused by the
committer rule.

| Product | What it needs | Release |
|---|---|---|
| FerroBRIDGE | Writes to the CDR under the facade caller's exchanged token (FerroHEALTH/FerroBRIDGE#1582) | FerroBRIDGE v0.0.23 |
| FerroFED | The gateway reaches the CDR under the caller (FerroHEALTH/FerroFED#1615) | FerroFED v0.0.17 |

FerroFED v0.0.17 follows this release, so a deployment with a FerroFED
gateway adds its `persons` issuers only once that release is out.
