# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1

# The English catalog of the parties group: the reference catalog and the
# one every lookup falls back to. Each message is one `PartiesMsg` of the
# crate's `locale::parties` module; a catalog of another language holds the
# same messages in a directory named by its BCP 47 tag beside this one.

## Numbers: the marks FerroUI's formatter groups a number with

number-group = ,
number-decimal = .

## Parties

parties-subtitle = The people, organisations, groups, agents and roles the CDR holds.
parties-note = Parties follow the released RM 1.1.0 demographic model; the REST API for them is still in development, and FerroEHR adds the relationship routes and the AQL over parties it does not define yet.
parties-roots = Kinds of party
parties-root-all = All
parties-column-name = Name
parties-column-committed = Last committed
parties-find-label = Find by a name or identifier (matching case)
parties-find-chip = Holding “{ $text }”
parties-find-capped = { $count ->
    [one] Only the first match is shown: type more to narrow the find.
   *[other] Only the first { $count } matches are shown: type more to narrow the find.
}
parties-pages = Pages of parties
parties-empty = The CDR holds no party of this kind yet.
parties-no-match = No party's identities hold this text.
parties-list-refused = The parties could not be listed.
parties-list-unserved = This CDR runs no AQL over parties, so open a party by its uid below.
parties-uid = Party uid
parties-open-refused = The party could not be opened.
parties-none-holds = No party has this uid.
parties-no-kind-holds = No party of any kind has this uid.
parties-tags-title = Tags on parties
parties-tag-index = The demographic tag index
parties-tag-on = On
parties-no-tags = No party carries a tag yet.
parties-tags-refused = The tags on parties could not be read.
