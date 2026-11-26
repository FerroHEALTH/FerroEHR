// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the access group, in the catalogs under `locales/access/`.

crate::locale::messages! {
    /// A string the access group draws, by its message in its catalog.
    AccessMsg in "./locales/access" {
        /// `The person themself`
        PartySelf = "access-party-self",
        /// `The access log the CDR answered is not JSON.`
        LogJson = "access-log-json",
        /// `Access log`
        Tab = "access-tab",
        /// `Access log of EHR { $id }`
        Title = "access-title",
        /// `Who accessed this record, when, what they reached and what the access rested …`
        Intro = "access-intro",
        /// `Entries of the access log`
        Caption = "access-caption",
        /// `Pages of the access log`
        Pages = "access-pages",
        /// `The access log holds no entry for this record yet.`
        Empty = "access-empty",
        /// `No entry matches these filters.`
        NoMatch = "access-no-match",
        /// `The access log could not be read.`
        Refused = "access-refused",
        /// `Your account may not read this access log. The person the record is about, t …`
        Forbidden = "access-forbidden",
        /// `This CDR serves no access log the viewer can read for this record.`
        Unserved = "access-unserved",
        /// `The CDR keeps no access log, as its local audit store is off.`
        StoreOff = "access-store-off",
        /// `When`
        ColumnWhen = "access-column-when",
        /// `Accessor`
        ColumnAccessor = "access-column-accessor",
        /// `Action`
        ColumnAction = "access-column-action",
        /// `Reached`
        ColumnReached = "access-column-reached",
        /// `Outcome`
        ColumnOutcome = "access-column-outcome",
        /// `Not named`
        Unknown = "access-unknown",
        /// `Action`
        FilterAction = "access-filter-action",
        /// `Outcome`
        FilterOutcome = "access-filter-outcome",
        /// `Accessor`
        FilterAccessor = "access-filter-accessor",
        /// `Ground`
        FilterJustified = "access-filter-justified",
        /// `Recorded`
        FilterWhen = "access-filter-when",
        /// `Include the person's own reads of this log`
        FilterSelf = "access-filter-self",
        /// `Any`
        FilterAny = "access-filter-any",
        /// `Action: { $value }`
        ChipAction = "access-chip-action",
        /// `Outcome: { $value }`
        ChipOutcome = "access-chip-outcome",
        /// `Accessor: { $value }`
        ChipAccessor = "access-chip-accessor",
        /// `Ground: { $value }`
        ChipJustified = "access-chip-justified",
        /// `With the person's own reads`
        ChipSelf = "access-chip-self",
        /// `Create`
        ActionCreate = "access-action-create",
        /// `Read`
        ActionRead = "access-action-read",
        /// `Update`
        ActionUpdate = "access-action-update",
        /// `Delete`
        ActionDelete = "access-action-delete",
        /// `Query`
        ActionQuery = "access-action-query",
        /// `Granted`
        OutcomeGranted = "access-outcome-granted",
        /// `Refused`
        OutcomeRefused = "access-outcome-refused",
        /// `Failed`
        OutcomeFailed = "access-outcome-failed",
        /// `A person`
        KindPerson = "access-kind-person",
        /// `An external person`
        KindExternal = "access-kind-external",
        /// `A delegated person`
        KindDelegated = "access-kind-delegated",
        /// `A representative`
        KindRepresentative = "access-kind-representative",
        /// `The person themself`
        KindSubject = "access-kind-subject",
        /// `An organisation`
        KindOrganisation = "access-kind-organisation",
        /// `A system`
        KindSystem = "access-kind-system",
        /// `On a justification`
        GroundJustification = "access-ground-justification",
        /// `Emergency access`
        GroundEmergency = "access-ground-emergency",
        /// `Access of { $when }`
        EntryTitle = "access-entry-title",
        /// `One entry of the access log, as the CDR recorded it.`
        EntrySubtitle = "access-entry-subtitle",
        /// `The access`
        EntryFacts = "access-entry-facts",
        /// `What it rested on`
        EntryGrounds = "access-entry-grounds",
        /// `What it reached`
        EntryReached = "access-entry-reached",
        /// `As the CDR served it`
        EntryAsServed = "access-entry-as-served",
        /// `No entry of this access log has this uid.`
        EntryGone = "access-entry-gone",
        /// `Operation`
        FactOperation = "access-fact-operation",
        /// `Reason`
        FactReason = "access-fact-reason",
        /// `Kind of accessor`
        FactKind = "access-fact-kind",
        /// `On behalf of`
        FactProvider = "access-fact-provider",
        /// `Roles`
        FactRoles = "access-fact-roles",
        /// `Purposes of use`
        FactPurposes = "access-fact-purposes",
        /// `Reason stated`
        FactStatedReason = "access-fact-stated-reason",
        /// `The entry names no authorisation.`
        GroundsNone = "access-grounds-none",
        /// `The entry names no object.`
        ReachedNone = "access-reached-none",
        /// `Ground`
        ColumnGround = "access-column-ground",
        /// `Name`
        ColumnName = "access-column-name",
        /// `Policy version`
        ColumnPolicyVersion = "access-column-policy-version",
        /// `Type`
        ColumnType = "access-column-type",
        /// `Object`
        ColumnObject = "access-column-object",
        /// `Categories`
        ColumnCategories = "access-column-categories",
        /// `Open this record on a justification`
        JustifyTitle = "access-justify-title",
        /// `The CDR asks why you open this record: no care relationship of yours covers  …`
        JustifyIntro = "access-justify-intro",
        /// `Purpose of use`
        JustifyPurpose = "access-justify-purpose",
        /// `Why you open this record`
        JustifyReason = "access-justify-reason",
        /// `Your own words, 1 to 1000 characters. The person reads them in their access  …`
        JustifyReasonHelp = "access-justify-reason-help",
        /// `Open on this justification`
        JustifySubmit = "access-justify-submit",
        /// `It holds for this record for 15 minutes, until you end it, or until you sign …`
        JustifyHolds = "access-justify-holds",
        /// `Justification stated`
        JustifyKept = "access-justify-kept",
        /// `Justification ended`
        JustifyEnded = "access-justify-ended",
        /// `The session store could not keep the justification.`
        JustifyStoreRefused = "access-justify-store-refused",
        /// `The CDR asks for a reason but names no header to state it in, so the viewer  …`
        JustifyUnoffered = "access-justify-unoffered",
        /// `Choose a purpose of use.`
        JustifyPurposeMissing = "access-justify-purpose-missing",
        /// `Give a reason of 1 to 1000 characters.`
        JustifyReasonLength = "access-justify-reason-length",
        /// `A reason holds no control characters.`
        JustifyReasonControl = "access-justify-reason-control",
        /// `You opened this record on a justification ({ $purpose }) until { $until }. Y …`
        JustifyBanner = "access-justify-banner",
        /// `End now`
        JustifyEnd = "access-justify-end",
    }
}
