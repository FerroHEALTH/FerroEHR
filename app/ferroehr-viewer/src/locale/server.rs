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
        /// `the clinical and demographic domains share one database credential`
        ClinicalDemographicDomainsShareOne = "clinical-demographic-domains-share-one",
        /// `text/csv; charset=utf-8`
        TextCsvCharsetUtf8 = "text-csv-charset-utf-8",
        /// `parameters must be a JSON object`
        ParametersMustJsonObject = "parameters-must-json-object",
        /// no stored query to run — open one from the stored-query list
        NoStoredQueryRunOpenOne = "no-stored-query-run-open-one",
        /// `the stored query needs a name`
        StoredQueryNeedsName = "stored-query-needs-name",
        /// column `$value0`
        Column2 = "column-2",
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
        /// `no subject pseudonym namespace is declared, so an EHR subject may carry a real identifier`
        NoSubjectPseudonymNamespaceDeclaredSo = "no-subject-pseudonym-namespace-declared-so",
        /// `an access record the audit queue cannot take is dropped and the request still succeeds`
        AccessRecordAuditQueueCannotTake = "access-record-audit-queue-cannot-take",
        /// `an EHR without its own access settings admits every caller the role checks let through`
        EhrWithoutOwnAccessSettingsAdmits = "ehr-without-own-access-settings-admits",
        /// `role checks are off, so every signed-in caller may use every enabled surface, the...`
        RoleChecksOffSoEverySigned = "role-checks-off-so-every-signed",
        /// `` `$version` is not a version to store at: a stored-query version is `major.minor.patch`... ``
        NotVersionStoreStoredQueryVersion = "not-version-store-stored-query-version",
    }
}
