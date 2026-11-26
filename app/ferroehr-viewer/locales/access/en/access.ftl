# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the access group: the access log and the access
# decisions the viewer draws. It is the reference catalog and the one every
# lookup falls back to. Each message is one `AccessMsg` of the crate's
# `locale::access` module; a catalog of another language holds the same
# messages in a directory named by its BCP 47 tag beside this one.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

## The access log

access-party-self = The person themself
access-log-json = The access log the CDR answered is not JSON.
access-tab = Access log
access-title = Access log of EHR { $id }
access-intro = Who accessed this record, when, what they reached and what the access rested on, as the CDR recorded it. The CDR decides what you read: the person and their representative read their own view, a reviewer the full record.
access-caption = Entries of the access log
access-pages = Pages of the access log
access-empty = The access log holds no entry for this record yet.
access-no-match = No entry matches these filters.
access-refused = The access log could not be read.
access-forbidden = Your account may not read this access log. The person the record is about, their representative and a reviewer read it.
access-unserved = This CDR serves no access log the viewer can read for this record.
access-store-off = The CDR keeps no access log, as its local audit store is off.
access-column-when = When
access-column-accessor = Accessor
access-column-action = Action
access-column-reached = Reached
access-column-outcome = Outcome
access-unknown = Not named
access-filter-action = Action
access-filter-outcome = Outcome
access-filter-accessor = Accessor
access-filter-justified = Ground
access-filter-when = Recorded
access-filter-self = Include the person's own reads of this log
access-filter-any = Any
access-chip-action = Action: { $value }
access-chip-outcome = Outcome: { $value }
access-chip-accessor = Accessor: { $value }
access-chip-justified = Ground: { $value }
access-chip-self = With the person's own reads
access-action-create = Create
access-action-read = Read
access-action-update = Update
access-action-delete = Delete
access-action-query = Query
access-outcome-granted = Granted
access-outcome-refused = Refused
access-outcome-failed = Failed
access-kind-person = A person
access-kind-external = An external person
access-kind-delegated = A delegated person
access-kind-representative = A representative
access-kind-subject = The person themself
access-kind-organisation = An organisation
access-kind-system = A system
access-ground-justification = On a justification
access-ground-emergency = Emergency access
access-entry-title = Access of { $when }
access-entry-subtitle = One entry of the access log, as the CDR recorded it.
access-entry-facts = The access
access-entry-grounds = What it rested on
access-entry-reached = What it reached
access-entry-as-served = As the CDR served it
access-entry-gone = No entry of this access log has this uid.
access-fact-operation = Operation
access-fact-reason = Reason
access-fact-kind = Kind of accessor
access-fact-provider = On behalf of
access-fact-roles = Roles
access-fact-purposes = Purposes of use
access-fact-stated-reason = Reason stated
access-grounds-none = The entry names no authorisation.
access-reached-none = The entry names no object.
access-column-ground = Ground
access-column-name = Name
access-column-policy-version = Policy version
access-column-type = Type
access-column-object = Object
access-column-categories = Categories

## The review across EHRs and the log's integrity

access-review-title = Access log
access-review-subtitle = Every access to EHR data across the records, and the evidence that the log is whole.
access-review-views = Access log views
access-review-tab-records = Records
access-review-tab-integrity = Integrity
access-review-intro = The full record of each access, and the tombstone of each entry a law ordered removed. Open an entry from its EHR's access log.
access-review-not-reviewer = Your account is no reviewer of the access log. An account the admin role admits reads it.
access-review-admin-off = The CDR's admin API is off, so the review of the access log is not served.
access-review-filter-ehr = EHR id
access-review-filter-party = Accessing party id
access-review-filter-issuer = Credential issuer
access-review-filter-subject = Credential subject
access-review-filter-request = Request id
access-review-chip = { $name }: { $value }
access-column-ehr = EHR
access-tombstone-row = Removed { $when }
access-tombstone-grounds = Grounds: { $grounds }
access-integrity-intro = Each window of entries is sealed by a digest that follows the seal before it, and a signed checkpoint covers the log up to an entry. A verification recomputes them all.
access-checkpoint-title = The newest checkpoint
access-checkpoint-none = No checkpoint is stored yet. A checkpoint is signed with the OpenPGP key of the version-signing keyring.
access-checkpoint-sequence = Covers the log up to entry
access-checkpoint-digest = Digest
access-checkpoint-algorithm = Algorithm
access-checkpoint-sealed = Sealed
access-checkpoint-signer = Signed by
access-checkpoint-proof = Signature
access-checkpoint-sign = Sign a checkpoint now
access-checkpoint-signed = Checkpoint signed
access-checkpoint-sign-refused = The CDR signed no checkpoint.
access-checkpoint-refused = The checkpoint could not be read.
access-verify-title = Verification
access-verify-run = Verify the log
access-verify-intact = The log is intact against the checkpoint at entry { $sequence }.
access-verify-no-checkpoint = No checkpoint was there to verify against, so the log is not shown intact.
access-verify-findings = { $count ->
    [one] The verification found one failure.
   *[other] The verification found { $count } failures.
}
access-verify-refused = The verification did not run.
access-seals-title = Seals
access-seals-from = First entry id
access-seals-to = Last entry id
access-seals-show = Show seals
access-seals-none = No window seals an entry of this span yet.
access-seals-refused = The seals could not be read.
access-seals-caption = Seals of the windows that cover the span
access-column-window = Window
access-column-span = Entries
access-column-count = Count
access-column-digest = Digest
access-column-previous = Follows
access-column-folded = Folded
access-seal-span = { $first } to { $last }
access-erase-action = Erase
access-entry-more = More actions
access-erase-title = Erase this entry?
access-erase-body = Erase an entry only on an order of the law before its retention ends. A tombstone replaces it and keeps the grounds you give; the entry cannot be read again.
access-erase-terminology = Grounds: terminology id
access-erase-code = Grounds: code
access-erase-words = Grounds: in words
access-erase-confirm = Erase the entry
access-erased-toast = Entry erased
access-erase-refused = The CDR erased nothing.
access-erase-needs-grounds = Give the grounds as a terminology id and a code.
access-export-action = Export the log
