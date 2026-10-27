# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the queries group: the reference catalog and the
# one every lookup falls back to. Each message is one `QueriesMsg` of the
# crate's `locale::queries` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

desc = {" "}DESC
asc = {" "}ASC
did-not-parse-data-path = did not parse as a data path
query-saved = Query saved

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

queries-subtitle = The CDR's stored queries, grouped by the namespace of their name.
queries-unqualified = Unqualified
queries-group-count = { $names ->
    [one] 1 query
   *[other] { $names } queries
}{ $versions ->
    [one] {""}
   *[other] , { $versions } versions
}
queries-older-version = older version
queries-open-in-query = Open in Query
queries-delete-version = Delete this version…
queries-namespace-chip = Namespace { $value }
queries-refused = The stored queries could not be read.
queries-views = Views of this stored query
queries-more = More actions for the stored query
queries-copy-name = Copy name
queries-line = { $name } · version { $version }
queries-delete-named = Delete version { $version }
queries-delete-title = Delete { $name } version { $version }?
queries-delete-message = Removes this version from the CDR's stored queries for good, for every client; its other versions stay.
queries-delete-confirm = Delete version { $version }
queries-deleted = Stored query version deleted
queries-delete-refused = The CDR refused to delete the version.
queries-definition = Definition
queries-stored-refused = The stored query could not be read.
queries-reading-older = You are reading version { $version }; Latest runs { $latest }.
queries-read-version = Read { $version }
queries-type = Type
queries-saved = Saved
queries-aql-title = { $name } { $version }
queries-which-version = Which version runs
queries-param-help = Numbers and true or false are sent as they are; quote a number to send it as text.
queries-fix-version = Nothing was sent: fix the version first.
queries-versions-intro = Each version is its own immutable definition; Latest runs the highest.

query-subtitle = Build AQL from a template, or write it; run it against the CDR.
query-run = Run
query-save = Save to the CDR
query-save-action = Save
query-namespace = Namespace
query-version-help = Empty writes the version the CDR assigns.
query-version-taken = This version of this name already exists, and a saved version cannot change.
query-save-refused = The CDR refused to store the query.
query-open-stored = Open it
query-build = Build
query-aql = AQL
query-aql-title = Query · AQL
query-ways = Ways to write the query
query-returns = Returns
query-shape-data = Data points
query-shape-compositions = Whole compositions
query-shape-count = A count
query-shape-ehrs = EHRs (cohort)
query-data-points = Data points
query-data-point = Data point
query-column-name = Column name
query-data-point-n = Data point { $number }
query-column-name-n = Column name of data point { $number }
query-remove-data-point = Remove the data point { $name }
query-add-data-point = Add a data point
queries-version-help = Latest sends no version; Prefix and Exact send the one typed here.
queries-sends = Sends
query-save-name-missing = Name the query to save it.
query-no-data-points = No data point yet; the query returns nothing until you add one.
query-conditions = Conditions
query-order-limit = Order and limit
query-order-by = Order by
query-no-order = No order
query-direction = Direction
query-newest-first = Newest first
query-oldest-first = Oldest first
query-limit = Row limit
query-bad-limit = The row limit is not a whole number.
query-it-builds = The AQL it builds
query-built-aql = Built AQL
query-open-in-aql = Open in AQL
query-open-in-build = Open in Build
query-not-built = The builder cannot write this query yet: { $reason }
query-cannot-build = This AQL cannot be opened in Build: { $reason }
query-not-number = The value of { $path }, { $value }, is not a number.
query-unknown-path = The template has no data point at { $path }.
query-catalog-refused = The template's data points could not be read.
query-run-refused = The CDR refused the query.
query-the-aql = The AQL
query-opened-stored = Opened from the stored query { $name } { $version }.
query-stored-refused = The stored query could not be read.
query-build-made = The AQL Build made, as it runs; edits here leave Build behind.
query-nothing-to-run = Nothing to run: write a query first.
query-grammar-stops = Nothing was sent: the grammar check found an error.
query-grammar-heading = { $count ->
    [one] Grammar check · 1
   *[other] Grammar check · { $count }
}
query-unexpected = Unexpected { $token } here.
query-ended-early = The query ends before it is complete.
query-parameters = Parameters
query-param-name-n = Name of parameter { $number }
query-param-value-n = Value of parameter { $number }
query-remove-param = Remove the parameter { $name }
query-add-param = Add a parameter
query-no-params = No parameter.

results-title = Results
results-chart-label = The numeric columns over the results
results-chart-summary = { $series } over { $axis }.
results-axis = The x axis is { $axis }, at its true spacing.
results-not-run = Run the query to see its rows here.
results-window = { $count ->
    [one] 1 row. No LIMIT in the query, so the CDR returns at most its own page.
   *[other] { $count } rows. No LIMIT in the query, so the CDR returns at most its own page.
}
results-window-own = { $count ->
    [one] 1 row, inside the query's own LIMIT.
   *[other] { $count } rows, inside the query's own LIMIT.
}
