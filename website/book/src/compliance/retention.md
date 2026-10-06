# Retention, restriction and objection

Three marks sit beside the clinical content, and none of them is an openEHR
concept. The specifications say what a repository must keep and how versions
relate to each other; they say nothing about a period after which data should
go, about limiting the processing of a record that stays, or about a patient
who does not want their data used for research. The law says all three, so
FerroEHR carries them as its own extension and this page is where they are
written down.

Nothing here deletes anything. A record in an openEHR repository is indelible
by design — a deletion commits a new version whose content is removed, and the
history stays — so a retention period in FerroEHR produces a **list**, and
acting on that list is the controller's decision, taken record by record and
carried out through
[physical deletion](../operations-admin-apis.md#physical-deletion).

## The retention register

Two tables and a view in the clinical domain, all driven through the admin API.

`retention_policy` is the register: one row per content category per
jurisdiction, carrying the period, what the period is measured from, and the
legal citation the period rests on.

| Column | Meaning |
|---|---|
| `kind` | The content category: `COMPOSITION`, `EHR_STATUS`, `FOLDER`, or `EHR` for the whole record |
| `category` | Optional: the EHDS priority category the period is keyed on (`patient-summary`, `eprescription`, `edispensation`, `imaging`, `test-results`, `discharge-report`, `national:<code>` or `none`); empty keys the period on every object of the kind |
| `jurisdiction` | ISO 3166-1 alpha-2, the same key the [identifier scanner](../installation/config-privacy.md) rules use |
| `period` | How long content of that category is kept |
| `anchor` | What the period is measured from: `last_commit`, `death`, or `majority` |
| `source` | The citation the period comes from, quoted as the act spells it |

`retention_anchor` is the per-EHR half: the jurisdiction whose periods apply,
the anchor instant once the deployment knows it, and any hold that suspends
disposal for the whole record with the reason it was placed.

`retention_due` joins the two and lists what has run out. Each row names the
EHR, the kind, the priority category where the period is keyed on one, the
citation, when the period expired, and two counts: the objects that are due,
and the objects exempted by a per-object hold.

Ship the register empty and declare the periods your organisation is subject
to. The periods are not the software's to choose — they follow from the law the
deployment runs under, and two of the provisions in the [compliance
overview](index.md) ask for exactly this register: GDPR Art. 30(1)(f) wants the
envisaged time limits for erasure per category of data in the record of
processing activities, and Swiss DSG Art. 25 Abs. 2 lit. d gives the subject a
right to be told the retention period or the criteria that fix it.

### Declaring a period

```http
PUT {base}/admin/retention/policy
Content-Type: application/json

{
  "kind": "EHR",
  "jurisdiction": "CH",
  "period": "20 years",
  "anchor": "last_commit",
  "source": "EPDV Art. 10 Abs. 1 lit. d"
}
```

**204** on success, replacing any earlier period for that kind, category and
jurisdiction. An unknown kind or category, an unknown anchor rule or a
non-positive period is **400**. `GET {base}/admin/retention/policy` reads the
whole register back, which is what an access answer and an audit both cite.

### Keying a period on a priority category

EHDS Annex II 3.4 asks the storing components to support retention periods
"that take into account the origins and categories of electronic health data".
Add `"category": "test-results"` to the body above and the period covers only
the objects your [`[audit.categories]`](../installation/config-audit.md#auditcategories)
map places in that category. The server mirrors the map into the clinical
domain at every boot, so the due list reads the map in force. An object's
category comes from its latest version that carries content: the template id
first, then the root archetype id; every kind other than `COMPOSITION` reads as
`none`. An object the map cannot classify is listed under the longest period
configured for its kind, so a missing map entry can only keep content longer. A
category row with no object is left out of the due list, and a period without a
category lists every object of its kind.

### Anchoring an EHR and holding it

```http
PUT {base}/admin/retention/anchor
Content-Type: application/json

{
  "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
  "jurisdiction": "CH",
  "anchored_at": "2026-01-31T00:00:00Z"
}
```

Leave `anchored_at` out until the anchor event is known; nothing is due without
it. Add `hold_at` and `hold_ground` together to suspend disposal for the whole
record: a hold with no stated reason is refused with **400**, because a hold
nobody can account for is not a record of anything.

A hold at object grain is a separate route, and it is the shape Swiss EPDV
Art. 10 Abs. 2 lit. b asks for, where the patient may ask that named data be
exempted from the destruction the same article otherwise requires:

```http
POST {base}/admin/retention/hold
Content-Type: application/json

{ "vo_id": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21", "held": true }
```

### Reading what is due

```http
GET {base}/admin/retention/due?limit=100
```

Oldest first. Every row carries `objects_due` and `objects_held`, so a
controller acting on the list disposes of the first and leaves the second where
they are. An EHR under a whole-record hold does not appear at all.

## Restriction of processing

GDPR Art. 4(3) defines restriction as marking stored data to limit its future
processing, and Art. 18(2) says what is left once the mark is set: storage, and
nothing else without the subject's consent or one of the named exceptions.
FerroEHR implements that literally, at two grains — a whole EHR, or one
versioned object inside it.

A restricted object stays in storage untouched, and every path that would
process it stops:

- a point read, a versioned read and a revision history answer **403** with a
  body naming the restriction — distinct from the authorization **403**, because
  the same caller with the same rights is refused until the restriction is
  lifted;
- AQL results omit it, at every scope, so naming the `ehr_id` does not make the
  content answerable;
- an [EHR Extract](../beyond-core/messaging.md#exporting-an-ehr) omits a
  restricted object, and refuses outright for a restricted EHR;
- the [change-event stream](../beyond-core/amqp.md) neither emits nor delivers
  an event about it;
- a write to it is refused, and nothing already stored changes.

The restriction refusal is the last gate on a write. The checks that need no
transaction run first: a body the template refuses is **422**, and a write to an
EHR whose `is_modifiable` is false, or a second directory for one EHR, is
**409**. Only the commit transaction itself reads the mark and answers **403**.
The order is deliberate. Validation stays outside the transaction so the
transaction is short, and the mark is read inside it so a concurrent lift either
commits before the write and is seen, or waits for it. A malformed write to a
restricted object therefore answers 422 rather than 403: the request is refused
either way and nothing stored changes, but the status names the gate that
stopped the write, not whether the restriction is in force.
`GET {base}/admin/restriction?ehr_id=…` answers that.

```http
POST {base}/admin/restriction
Content-Type: application/json

{
  "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
  "vo_id": "df58b2ee-30bd-4b2c-9b7d-3a0f8e5c6d21",
  "ground": "gdpr-18-1-a",
  "note": "accuracy contested on 2026-09-15"
}
```

Omit `vo_id` to restrict the whole record. `ground` is one of the four
Art. 18(1) points — `gdpr-18-1-a` through `gdpr-18-1-d` — or `national` for a
member-state rule that goes beyond them, which is how German BDSG § 35 Abs. 2
and Abs. 3 are carried.

`POST {base}/admin/restriction/lift` with the same grain lifts it. The register
keeps the request and stamps it lifted rather than removing it, because
Art. 18(3) makes the sequence itself the obligation: the subject is informed
**before** the restriction is lifted, and a register that forgot the request
could not show that anyone was. `GET {base}/admin/restriction?ehr_id=…` reads
the whole sequence back. Lifting a whole-record restriction leaves an
object-scoped one standing on its own.

## Objection to research processing

GDPR Art. 21(6) gives the subject a right to object to processing for
scientific or historical research or statistical purposes under Art. 89(1),
unless the processing is necessary for a task carried out for reasons of public
interest. That is narrower than Art. 18: it reaches research, not care.

While an objection stands, the EHR is absent from every full-population AQL
query, from every export, and from the change-event stream — the three surfaces
a secondary-use consumer reads this repository through. A query that names the
`ehr_id` and a read of the record by the treating clinician are untouched.

```http
POST {base}/admin/research-objection
Content-Type: application/json

{ "ehr_id": "7d44b88c-4199-4bad-97dc-d78268e01398", "objected": true }
```

Send `"ground": "…"` beside `"objected": true` to record the public-interest
ground the article admits for an override: the EHR returns to the research
population and the ground says on whose authority. Send `"objected": false` to
withdraw the objection entirely; a withdrawal carries no ground, because there
is nothing left to override.

## The audit trail, and the access-log ceiling

Setting and lifting any of these marks is an access record of its own, naming
the EHR, the object where the act was object-scoped, and which register moved.
These are the acts a supervisory authority asks about after the fact, so they
are in the trail beside the reads and writes.

The trail has a retention question of its own, and it runs the other way:
the law sets a **floor** below which an access log may not be reaped (three
years from each date of access in every EU Member State under EHDS Art. 9(2),
five years in the Netherlands, one year in Switzerland), and one national rule
sets a **ceiling**. SGB V § 309 Abs. 1 asks the controllers of a German
telematics-infrastructure application to keep access logs for the three-year
limitation period, and Abs. 3 requires deletion without delay once it has run.
That provision binds the controllers § 307 names, which no software can infer,
so the deployment declares it:

```toml
[audit.store]
retention_years = 3
sgb_v_309_controller = true
```

With the declaration in place, the server refuses at boot any horizon above the
ceiling, including `retention_days = 0` (keep forever), and refuses a
configuration whose floors and ceilings contradict each other outright, rather
than silently preferring one.

Germany is a Member State, so the EHDS floor of three years applies there as
well. Both rules are three calendar years, which is why the horizon above is
stated in years: `retention_years = 3` meets the floor and the ceiling exactly,
and the reaper deletes what is older than the same calendar date three years
ago. A day count cannot: three calendar years span 1095 or 1096 days, so a
floor in days has to be 1096 and a ceiling 1095, and with
`sgb_v_309_controller = true` the server refuses every `retention_days` value
and names `retention_years = 3` as the fix.
Retention of the trail itself is described in
[Audit](../audit.md#retention-and-who-chooses-it).

## What this page does not claim

The registers record decisions and make them checkable. They do not make a
deployment compliant with any of the provisions named above: the periods, the
grounds, the notifications, the weighing of an objection and the decision to
dispose of a record are all the controller's, and the [shared responsibility
page](shared-responsibility.md) says which side of the line each one falls on.
