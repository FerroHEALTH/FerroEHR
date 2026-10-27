# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the overview group: the reference catalog and the
# one every lookup falls back to. Each message is one `OverviewMsg` of the
# crate's `locale::overview` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

try-again = Try again
unknown-count = —
upload-a-template = Upload a template
column-namespace = Namespace
column-matches = Matches

stat-ehrs = EHRs
stat-compositions = Compositions, found in Query
stat-templates = Templates
stat-stored-queries = Stored queries
dashboard-subtitle = What { $host } holds, as it answered just now.
dashboard-no-answer = The CDR did not answer.
dashboard-refused = The CDR did not answer, so the dashboard cannot be shown. Your session is kept.
dashboard-per-day = Compositions per day
dashboard-per-day-summary = { $total } compositions from { $first } to { $last }, at most { $most } on one day.
dashboard-per-day-note = By commit time, over the newest compositions; read with one AQL query.
dashboard-per-day-refused = The compositions per day could not be read.
dashboard-no-compositions = No composition is committed yet.
dashboard-create-first-ehr = Create the first EHR
dashboard-by-namespace = Matches by namespace
dashboard-queries = { $count ->
    [one] 1 query
   *[other] { $count } queries
}
dashboard-matches = { $count ->
    [one] 1 match
   *[other] { $count } matches
}
dashboard-no-total = No total
dashboard-namespaces-refused = The stored queries could not be read.

find-title = Find
find-subtitle = One answer for an id, a subject, a template or a stored query.
find-empty = Type an id, a subject, a template or a query name in the search field above.
find-no-id-holds = No EHR, composition or party has the id { $id }.
find-nothing-for = Nothing matches “{ $typed }”.
find-matches = { $count ->
    [one] 1 match for “{ $typed }”
   *[other] { $count } matches for “{ $typed }”
}
find-other-ways = Other ways to find a record
find-compositions = Compositions
find-ehr = EHR { $id }
find-ehr-line = Created { $created }
find-composition-line = In EHR { $ehr }
find-party-line = { $kind }
find-query-line = Stored query, version { $version }
find-template-line = Template: { $concept }
find-nothing-title = Nothing found
find-nothing-line = Check the spelling, or open the list of the kind of record you look for.
find-unfinished-title = Part of the search did not finish
find-unfinished-line = The CDR refused the search over { $kinds }; the answer may be incomplete.
find-way-ehr = EHRs
find-way-ehr-line = Every EHR, with the subject search.
find-way-subject = By subject
find-way-subject-line = An EHR by its subject id and namespace.
find-way-party = Parties
find-way-party-line = A party by its id or its tags.
