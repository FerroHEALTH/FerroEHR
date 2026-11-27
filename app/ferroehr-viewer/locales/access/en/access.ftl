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
access-own-view = This is the person's own view of their access log, as the CDR serves it to them and their representative. Their own reads of it are left out unless you include them.
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

## Justified and emergency access

access-justify-title = Open this record on a justification
access-justify-intro = The CDR refused this record: no care relationship of yours covers it. If the access policy lets you, open it on a justification. The purpose you choose decides, as the policy says, whether this is a justified or an emergency access.
access-justify-purpose = Purpose of use
access-justify-reason = Why you open this record
access-justify-reason-help = Your own words, 1 to 1000 characters. The person reads them in their access log.
access-justify-submit = Open on this justification
access-justify-holds = It holds for this record for 15 minutes, until you end it, or until you sign out.
access-justify-kept = Justification stated
access-justify-ended = Justification ended
access-justify-store-refused = The session store could not keep the justification.
access-justify-unoffered = The CDR asks for a reason but names no header to state it in, so the viewer cannot offer a justification.
access-justify-purpose-missing = Choose a purpose of use.
access-justify-reason-length = Give a reason of 1 to 1000 characters.
access-justify-reason-control = A reason holds no control characters.
access-justify-banner = You opened this record on a justification ({ $purpose }) until { $until }. Your reason: { $reason }
access-justify-end = End now
