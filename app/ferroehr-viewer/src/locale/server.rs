// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the server alone, in the catalogs under `locales/server/`.
//!
//! Only code the browser never runs looks them up, so their catalog is not
//! compiled for the browser at all (`crate::locale`).

crate::locale::messages! {
    /// A string the server alone draws, by its message in its catalog.
    ServerMsg in "./locales/server" {
        /// `no EHR id to delete`
        NoEhrIdDelete = "no-ehr-id-delete",
        /// `a stored-query delete needs both a qualified name and a version`
        StoredQueryDeleteNeedsBothQualified = "stored-query-delete-needs-both-qualified",
        /// `no template id to delete`
        NoTemplateIdDelete = "no-template-id-delete",
        /// `Row order`
        RowOrder = "row-order",
        /// `multimedia files are stored over a connection without encryption`
        MultimediaFilesStoredOverConnectionWithout = "multimedia-files-stored-over-connection-without",
        /// `access records are sent to the audit repository without encryption`
        AccessRecordsSentAuditRepositoryWithout = "access-records-sent-audit-repository-without",
        /// `change events are published to a message broker without encryption`
        ChangeEventsPublishedMessageBrokerWithout = "change-events-published-message-broker-without",
        /// `a listener serves plain HTTP on an address reachable from the network`
        ListenerServesPlainHttpAddressReachable = "listener-serves-plain-http-address-reachable",
        /// `requests are served without authentication`
        RequestsServedWithoutAuthentication = "requests-served-without-authentication",
        /// `schema preparation runs on the clinical runtime credential`
        SchemaPreparationRunsClinicalRuntimeCredential = "schema-preparation-runs-clinical-runtime-credential",
        /// `every access is recorded without the category of the data it reached`
        AccessRecordedWithoutCategoryData = "access-recorded-without-category-data",
        /// `access is not recorded to a durable audit trail`
        AccessNotRecordedDurableAuditTrail = "access-not-recorded-durable-audit-trail",
        /// `two domains reach the same PostgreSQL cluster`
        TwoDomainsReachSamePostgresqlCluster = "two-domains-reach-same-postgresql-cluster",
        /// `the clinical, demographic and linkage domains share one database credential`
        ClinicalDemographicLinkageDomainsShareOne = "clinical-demographic-linkage-domains-share-one",
        /// `text/csv; charset=utf-8`
        TextCsvCharsetUtf8 = "text-csv-charset-utf-8",
        /// `parameters must be a JSON object`
        ParametersMustJsonObject = "parameters-must-json-object",
        /// no stored query to run — open one from the stored-query list
        NoStoredQueryRunOpenOne = "no-stored-query-run-open-one",
        /// `the stored query needs a name`
        StoredQueryNeedsName = "stored-query-needs-name",
        /// `$object is not in the CDR any more: it was already deleted. Reload this screen.`
        NotCdrAnyMoreWasAlready = "not-cdr-any-more-was-already",
        /// The CDR rejected the delete of `$object`: `$message`.
        CdrRejectedDelete = "cdr-rejected-delete",
        /// Deleting `$object` failed: `$error`
        DeletingFailed = "deleting-failed",
        /// The viewer session expired before `$object` was deleted — sign in again and retry.
        ViewerSessionExpiredBeforeWasDeleted = "viewer-session-expired-before-was-deleted",
        /// Deleting `$object` failed: `$other`
        DeletingFailed2 = "deleting-failed-2",
        /// column `$value0`
        Column2 = "column-2",
        /// Open: `$gaps`.
        Open2 = "open-2",
        /// export failed: `$e`
        ExportFailed = "export-failed",
        /// parameters must be a JSON object: `$e`
        ParametersMustJsonObject2 = "parameters-must-json-object-2",
        /// unknown export format ``$other`` (csv | json)
        UnknownExportFormatCsvJson = "unknown-export-format-csv-json",
        /// `unparseable RESULT_SET: '$e'`
        UnparseableResultSet = "unparseable-result-set",
        /// column `$value0`
        Column3 = "column-3",
        /// attachment; filename="`$filename`"
        AttachmentFilename = "attachment-filename",
        /// `$object was not sent: $message. Correct the input and retry.`
        WasNotSentCorrectInputRetry = "was-not-sent-correct-input-retry",
        /// The CDR rejected `$object`: `$message`. Fix the reported problem and retry.
        CdrRejectedFixReportedProblemRetry = "cdr-rejected-fix-reported-problem-retry",
        /// The CDR answered `$status` to `$object`: `$message`. Nothing was saved.
        CdrAnsweredNothingWasSaved = "cdr-answered-nothing-was-saved",
        /// parameters must be a JSON object: `$e`
        ParametersMustJsonObject3 = "parameters-must-json-object-3",
        /// stored-query list JSON: `$e`
        StoredQueryListJson = "stored-query-list-json",
        /// stored query JSON: `$e`
        StoredQueryJson = "stored-query-json",
        /// parameters must be a JSON object: `$e`
        ParametersMustJsonObject4 = "parameters-must-json-object-4",
        /// stored-query result JSON: `$e`
        StoredQueryResultJson = "stored-query-result-json",
        /// conformance manifest JSON: `$e`
        ConformanceManifestJson = "conformance-manifest-json",
        /// `$object is still referenced by committed data, so the CDR refused the delete...`
        StillReferencedCommittedDataSoCdr = "still-referenced-committed-data-so-cdr",
        /// This session may not delete `$object` (`$message`). Sign in with an ADMIN-role account...
        SessionMayNotDeleteSignAdmin = "session-may-not-delete-sign-admin",
        /// The CDR no longer accepts this session, so `$object` was not deleted (`$message`)....
        CdrNoLongerAcceptsSessionSo = "cdr-no-longer-accepts-session-so",
        /// `Sandbox deployment: this deployment has not made the production separations and must...`
        SandboxDeploymentDeploymentHasNotMade = "sandbox-deployment-deployment-has-not-made",
        /// `no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier`
        NoSubjectPseudonymNamespaceDeclaredSo = "no-subject-pseudonym-namespace-declared-so",
        /// `an access record the audit queue cannot take is dropped and the request still succeeds`
        AccessRecordAuditQueueCannotTake = "access-record-audit-queue-cannot-take",
        /// `an EHR without its own access settings admits every caller the role checks let through`
        EhrWithoutOwnAccessSettingsAdmits = "ehr-without-own-access-settings-admits",
        /// `role checks are off, so every signed-in caller may use every enabled surface, the...`
        RoleChecksOffSoEverySigned = "role-checks-off-so-every-signed",
        /// `$object changed in the CDR since this screen loaded ($message). Reload the latest...`
        ChangedCdrSinceScreenLoadedReload = "changed-cdr-since-screen-loaded-reload",
        /// The CDR already holds a conflicting object, so `$object` was not created (`$message`)....
        CdrAlreadyHoldsConflictingObjectSo = "cdr-already-holds-conflicting-object-so",
        /// `$object is not in the CDR ($message): it may have been deleted meanwhile. Reload this...`
        NotCdrMayHaveBeenDeleted = "not-cdr-may-have-been-deleted",
        /// The CDR no longer accepts this session, so `$object` was not saved (`$message`). Sign...
        CdrNoLongerAcceptsSessionSo2 = "cdr-no-longer-accepts-session-so-2",
        /// This session may not write `$object` (`$message`). Sign in with an account that...
        SessionMayNotWriteSignAccount = "session-may-not-write-sign-account",
        /// The viewer session expired before `$object` was saved — sign in again and retry; your...
        ViewerSessionExpiredBeforeWasSaved = "viewer-session-expired-before-was-saved",
        /// The CDR is unreachable (`$message`), so `$object` was not saved. Check the CDR, then...
        CdrUnreachableSoWasNotSaved = "cdr-unreachable-so-was-not-saved",
        /// `$object could not be saved: $message. Retry; if it persists, check the viewer logs.`
        CouldNotSavedRetryIfPersists = "could-not-saved-retry-if-persists",
        /// `$object moved on in the CDR since this screen loaded, so nothing was deleted...`
        MovedCdrSinceScreenLoadedSo = "moved-cdr-since-screen-loaded-so",
        /// The CDR refused to delete `$object`: `$message`. It may already be deleted — reload...
        CdrRefusedDeleteMayAlreadyDeleted = "cdr-refused-delete-may-already-deleted",
        /// `$object is not in the CDR ($message): it may have been deleted meanwhile. Reload this...`
        NotCdrMayHaveBeenDeleted2 = "not-cdr-may-have-been-deleted-2",
        /// This session may not delete `$object` (`$message`). Sign in with an account that...
        SessionMayNotDeleteSignAccount = "session-may-not-delete-sign-account",
        /// The CDR no longer accepts this session, so nothing was deleted (`$message`). Sign in...
        CdrNoLongerAcceptsSessionSo3 = "cdr-no-longer-accepts-session-so-3",
        /// `` `$version` is not a version to store at: a stored-query version is `major.minor.patch`... ``
        NotVersionStoreStoredQueryVersion = "not-version-store-stored-query-version",
    }
}
