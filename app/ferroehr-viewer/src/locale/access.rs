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
        /// `Access log`
        ReviewTitle = "access-review-title",
        /// `Every access to EHR data across the records, and the evidence that the log i`
        ReviewSubtitle = "access-review-subtitle",
        /// `Access log views`
        ReviewViews = "access-review-views",
        /// `Records`
        ReviewTabRecords = "access-review-tab-records",
        /// `Integrity`
        ReviewTabIntegrity = "access-review-tab-integrity",
        /// `The full record of each access, and the tombstone of each entry a law ordere`
        ReviewIntro = "access-review-intro",
        /// `Your account is no reviewer of the access log. An account the admin role adm`
        ReviewNotReviewer = "access-review-not-reviewer",
        /// `The CDR's admin API is off, so the review of the access log is not served.`
        ReviewAdminOff = "access-review-admin-off",
        /// `EHR id`
        ReviewFilterEhr = "access-review-filter-ehr",
        /// `Accessing party id`
        ReviewFilterParty = "access-review-filter-party",
        /// `Credential issuer`
        ReviewFilterIssuer = "access-review-filter-issuer",
        /// `Credential subject`
        ReviewFilterSubject = "access-review-filter-subject",
        /// `Request id`
        ReviewFilterRequest = "access-review-filter-request",
        /// `{ $name }: { $value }`
        ReviewChip = "access-review-chip",
        /// `EHR`
        ColumnEhr = "access-column-ehr",
        /// `Removed { $when }`
        TombstoneRow = "access-tombstone-row",
        /// `Grounds: { $grounds }`
        TombstoneGrounds = "access-tombstone-grounds",
        /// `Each window of entries is sealed by a digest that follows the seal before it`
        IntegrityIntro = "access-integrity-intro",
        /// `The newest checkpoint`
        CheckpointTitle = "access-checkpoint-title",
        /// `No checkpoint is stored yet. A checkpoint is signed with the OpenPGP key of `
        CheckpointNone = "access-checkpoint-none",
        /// `Covers the log up to entry`
        CheckpointSequence = "access-checkpoint-sequence",
        /// `Digest`
        CheckpointDigest = "access-checkpoint-digest",
        /// `Algorithm`
        CheckpointAlgorithm = "access-checkpoint-algorithm",
        /// `Sealed`
        CheckpointSealed = "access-checkpoint-sealed",
        /// `Signed by`
        CheckpointSigner = "access-checkpoint-signer",
        /// `Signature`
        CheckpointProof = "access-checkpoint-proof",
        /// `Sign a checkpoint now`
        CheckpointSign = "access-checkpoint-sign",
        /// `Checkpoint signed`
        CheckpointSigned = "access-checkpoint-signed",
        /// `The CDR signed no checkpoint.`
        CheckpointSignRefused = "access-checkpoint-sign-refused",
        /// `The checkpoint could not be read.`
        CheckpointRefused = "access-checkpoint-refused",
        /// `Verification`
        VerifyTitle = "access-verify-title",
        /// `Verify the log`
        VerifyRun = "access-verify-run",
        /// `The log is intact against the checkpoint at entry { $sequence }.`
        VerifyIntact = "access-verify-intact",
        /// `No checkpoint was there to verify against, so the log is not shown intact.`
        VerifyNoCheckpoint = "access-verify-no-checkpoint",
        /// `{ $count ->`
        VerifyFindings = "access-verify-findings",
        /// `The verification did not run.`
        VerifyRefused = "access-verify-refused",
        /// `Seals`
        SealsTitle = "access-seals-title",
        /// `First entry id`
        SealsFrom = "access-seals-from",
        /// `Last entry id`
        SealsTo = "access-seals-to",
        /// `Show seals`
        SealsShow = "access-seals-show",
        /// `No window seals an entry of this span yet.`
        SealsNone = "access-seals-none",
        /// `The seals could not be read.`
        SealsRefused = "access-seals-refused",
        /// `Seals of the windows that cover the span`
        SealsCaption = "access-seals-caption",
        /// `Window`
        ColumnWindow = "access-column-window",
        /// `Entries`
        ColumnSpan = "access-column-span",
        /// `Count`
        ColumnCount = "access-column-count",
        /// `Digest`
        ColumnDigest = "access-column-digest",
        /// `Follows`
        ColumnPrevious = "access-column-previous",
        /// `Folded`
        ColumnFolded = "access-column-folded",
        /// `{ $first } to { $last }`
        SealSpan = "access-seal-span",
        /// `Erase`
        EraseAction = "access-erase-action",
        /// `More actions`
        EntryMore = "access-entry-more",
        /// `Erase this entry?`
        EraseTitle = "access-erase-title",
        /// `Erase an entry only on an order of the law before its retention ends. A tomb`
        EraseBody = "access-erase-body",
        /// `Grounds: terminology id`
        EraseTerminology = "access-erase-terminology",
        /// `Grounds: code`
        EraseCode = "access-erase-code",
        /// `Grounds: in words`
        EraseWords = "access-erase-words",
        /// `Erase the entry`
        EraseConfirm = "access-erase-confirm",
        /// `Entry erased`
        ErasedToast = "access-erased-toast",
        /// `The CDR erased nothing.`
        EraseRefused = "access-erase-refused",
        /// `Give the grounds as a terminology id and a code.`
        EraseNeedsGrounds = "access-erase-needs-grounds",
        /// `Export the log`
        ExportAction = "access-export-action",
    }
}
