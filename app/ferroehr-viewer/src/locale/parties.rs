// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the parties group, in the catalogs under `locales/parties/`.

crate::locale::messages! {
    /// A string the parties group draws, by its message in its catalog.
    PartiesMsg in "./locales/parties" {
        /// `The people, organisations, groups, agents and roles the CDR holds.`
        Subtitle = "parties-subtitle",
        /// `Parties follow the released RM 1.1.0 demographic model; …`
        Note = "parties-note",
        /// `Kinds of party`
        Roots = "parties-roots",
        /// `All`
        RootAll = "parties-root-all",
        /// `Name`
        ColumnName = "parties-column-name",
        /// `Last committed`
        ColumnCommitted = "parties-column-committed",
        /// `Find by a name or identifier (matching case)`
        FindLabel = "parties-find-label",
        /// `Holding “{ $text }”`
        FindChip = "parties-find-chip",
        /// `Only the first { $count } matches are shown: type more to narrow the find.`
        FindCapped = "parties-find-capped",
        /// `Pages of parties`
        Pages = "parties-pages",
        /// `The CDR holds no party of this kind yet.`
        Empty = "parties-empty",
        /// `No party's identities hold this text.`
        NoMatch = "parties-no-match",
        /// `The parties could not be listed.`
        ListRefused = "parties-list-refused",
        /// `This CDR runs no AQL over parties, so open a party by its uid below.`
        ListUnserved = "parties-list-unserved",
        /// `Party uid`
        Uid = "parties-uid",
        /// `The party could not be opened.`
        OpenRefused = "parties-open-refused",
        /// `No party has this uid.`
        NoneHolds = "parties-none-holds",
        /// `No party of any kind has this uid.`
        NoKindHolds = "parties-no-kind-holds",
        /// `Tags on parties`
        TagsTitle = "parties-tags-title",
        /// `The demographic tag index`
        TagIndex = "parties-tag-index",
        /// `On`
        TagOn = "parties-tag-on",
        /// `No party carries a tag yet.`
        NoTags = "parties-no-tags",
        /// `The tags on parties could not be read.`
        TagsRefused = "parties-tags-refused",
    }
}
