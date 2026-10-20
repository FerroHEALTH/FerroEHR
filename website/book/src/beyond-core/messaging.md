# EHR Extract & messaging

Moving a patient's record between openEHR systems (migrating to another CDR,
replicating an EHR to a downstream repository, or importing an externally
produced document) is what openEHR's EHR Extract and messaging services are
for. FerroEHR implements whole-EHR export, import into a new or an existing EHR
(including cross-system cloning that preserves version identity), and Template
Data Document (TDD) import.

> [!NOTE]
> The openEHR **service model** defines a Message component, but the released
> REST API publishes no extract, message, or TDD endpoint at all. FerroEHR
> therefore serves these operations under a `/message` group of its own design;
> the routes are ours, not the standard's, and they gate no openEHR conformance
> claim. Unlike the admin extensions, they are **not** admin-gated: they carry
> the same ordinary authentication as the clinical API. The six routes, with
> their bodies, parameters, and status codes, are in
> [Admin & messaging APIs](../operations-admin-apis.md#ehr-extract-and-tdd-import).

<!-- toc -->

## Exporting an EHR

An export produces an openEHR EXTRACT: a self-contained package of an EHR's
versioned objects. There are two ways to ask for one.

**Whole-EHR export** takes every versioned object in one EHR at its latest
version and assembles them into a single extract. This is the simplest way to
snapshot or hand off a complete record, and it needs nothing but the EHR's
identifier.

**Export by specification** takes an `EXTRACT_SPEC`: a manifest naming which
entities to include (each by EHR id or by subject id, optionally narrowed to
specific version containers) plus the extract type. It produces one extract
per manifest entity, in manifest order. Use it for selective or
policy-controlled export.

> [!TIP]
> The manifest must name at least one entity: `EXTRACT_MANIFEST.entities` is
> mandatory and non-empty in the Reference Model, so an empty manifest does not
> even decode: the request is refused as malformed rather than answered with an
> empty result. If you want "everything", that is whole-EHR export, not an empty
> spec.

Not every selector the Reference Model allows is supported: search criteria and
some commit-time intervals are refused explicitly rather than silently ignored,
so an export never quietly returns a different set than you asked for.

## Importing and cloning across systems

Import is the inverse, and it is where openEHR's distributed version identity
matters. When a record produced on one system is imported into another, the
imported versions keep their original identity while being recorded as having
arrived from elsewhere.

**Cloning a whole EHR** takes an extract and materializes it into a new EHR.
There are exactly two outcomes, and you choose between them with one optional
parameter:

- **Supply a target EHR id** and the clone lands under that identifier; the
  "same patient, other EHR service" case.
- **Supply nothing** and the source EHR id the extract carries is re-used, which
  makes it a true clone. The server does not invent a fresh identifier; if the
  extract names no source id and you named no target, the request is refused.

Either way, the response names the EHR that was created, so a caller that
supplied no id still learns what it got. An EHR that already exists under the
target id is a conflict, not a merge, and so is an imported `EHR_STATUS` naming
a subject some other EHR already holds.

Each original version in the extract is committed wrapped in an
`IMPORTED_VERSION`, so the record shows both the original authorship and the
fact of import: version identity is preserved, not regenerated.

**Importing into an existing EHR** merges an extract's versions into an EHR that
is already there, following the openEHR change-control copying rules. The
extract's content items become new versions of that EHR's versioned objects.

Together these are the mechanism behind cross-system migration: export from the
source, import into the destination, and the destination's history faithfully
reflects where each version came from.

**An import takes the data-minimisation pass too.** Extract import stores what
it receives, so it cannot rewrite a body; instead it refuses one whose content
carries a national identifier, a subject reference that is not a declared
pseudonym, or an identified party the deployment does not permit, with the same
`422` and the same RM path the ordinary commit path reports. The rules are on
[Privacy & data minimisation](../installation/config-privacy.md), and they
apply to the admin archive load for the same reason.

### Protected national identifiers do not travel in an extract

With [national-identifier protection](../installation/config-privacy.md#protecting-national-identifiers-in-the-demographic-domain)
on, a party's versioned body carries a reference where each protected
identifier stood, and the value lives in a sealed row under that deployment's
own root key. An extract carries the versions and never the sealed rows, so the
identifier cannot cross from one FerroEHR to another in an extract:

- **A party with a plaintext protected value is refused.** An imported version
  is a faithful copy that is never modified (RM common `master06` §Copying), so
  the receiving deployment cannot seal it on the way in.
- **A party with another deployment's sealed reference is refused.** The row it
  names is not held here, so it could never be read back, and the refusal
  (`422`) says so.

This is a decided refusal, not a gap waiting for a key exchange. Re-wrapping
the sealed rows for the receiving deployment would hand one organisation's key
material, or a wrapped copy of every value, to another, and no openEHR
specification defines such an exchange. Move a record this way instead:

1. Export the EHR and import the extract without the party. The clinical side
   holds pseudonyms and never a national identifier, so the clinical content
   moves unchanged.
2. Register the person in the receiving deployment through its own party API
   (`POST {base}/demographic/person`), with the identifier the receiving
   organisation holds for the person. The receiving deployment seals it under
   its own key.
3. Link the party to the EHR in the receiving deployment.

Between two deployments of **one** organisation that share the root key, the
admin [dump and load](../operations-admin-apis.md#dump-and-load) carries the
sealed rows as ciphertext and restores them.

When ATNA auditing is enabled (it is on by default, see
[Security](../security.md#atna-audit-trail)) each completed
export and import emits a security-audit event under the ATNA `Extract` object
class, so records moving between systems are captured in the audit trail with
their direction.

## Importing TDDs

A Template Data Document (TDD) is a template-shaped XML document carrying the
data for one composition. TDD import converts a TDD into a composition against
the operational template its root names and commits it, returning the new
version's object version id.

The template must already be provisioned through the definition API, and the
commit goes through the same validated write path as any other composition (see
[Templates & validation](../templates-validation.md)) so a malformed document,
an unknown EHR, or an unknown template is rejected rather than partially stored.

A batch variant imports several TDDs in one call and is **all-or-nothing**:
every document is converted before any is committed, so one unconvertible
document rejects the whole batch and commits nothing. An empty array is a
fulfilled no-op: the target EHR is still checked, and nothing is created.

## Divergent copies and version branches

openEHR's version tree branches when a version created on another system is
modified locally, and FerroEHR implements that: the local write forks a branch at
the imported version's fork point, and the new version id carries a three-part
tree id (`…::2.1.1`) rather than a plain trunk number.

Branch versions are served at the same URLs as trunk ones and appear in the
revision history and in `ALL_VERSIONS` queries; the *latest* version of an object
is always the latest trunk version. See
[Content negotiation & errors](../using-the-api/content-negotiation.md) for how
version ids read on the wire. So importing a copy of a record that has diverged
on two systems is supported, and the divergence stays visible in the version tree
instead of being flattened away.
