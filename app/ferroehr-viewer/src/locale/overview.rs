// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the overview group, in the catalogs under `locales/overview/`.
//!
//! Only code of the overview group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the overview group draws, by its message in its catalog.
    OverviewMsg in "./locales/overview" {
        /// `Matches`
        ColumnMatches = "column-matches",
        /// `Namespace`
        ColumnNamespace = "column-namespace",
        /// `Matches by namespace`
        DashboardByNamespace = "dashboard-by-namespace",
        /// `Create the first EHR`
        DashboardCreateFirstEhr = "dashboard-create-first-ehr",
        /// `{ $count ->`
        DashboardMatches = "dashboard-matches",
        /// `The stored queries could not be read.`
        DashboardNamespacesRefused = "dashboard-namespaces-refused",
        /// `The CDR did not answer.`
        DashboardNoAnswer = "dashboard-no-answer",
        /// `No composition is committed yet.`
        DashboardNoCompositions = "dashboard-no-compositions",
        /// `No total`
        DashboardNoTotal = "dashboard-no-total",
        /// `Compositions per day`
        DashboardPerDay = "dashboard-per-day",
        /// `By commit time, over the newest compositions; read with one AQL query.`
        DashboardPerDayNote = "dashboard-per-day-note",
        /// `The compositions per day could not be read.`
        DashboardPerDayRefused = "dashboard-per-day-refused",
        /// `{ $total } compositions from { $first } to { $last }, at most { $most } on one day.`
        DashboardPerDaySummary = "dashboard-per-day-summary",
        /// `{ $count ->`
        DashboardQueries = "dashboard-queries",
        /// `The CDR did not answer, so the dashboard cannot be shown. Your session is kept.`
        DashboardRefused = "dashboard-refused",
        /// `What { $host } holds, as it answered just now.`
        DashboardSubtitle = "dashboard-subtitle",
        /// `Compositions, found in Query`
        StatCompositions = "stat-compositions",
        /// `EHRs`
        StatEhrs = "stat-ehrs",
        /// `Stored queries`
        StatStoredQueries = "stat-stored-queries",
        /// `Templates`
        StatTemplates = "stat-templates",
        /// `Try again`
        TryAgain = "try-again",
        /// `—`
        UnknownCount = "unknown-count",
        /// `Upload a template`
        UploadATemplate = "upload-a-template",
        /// `In EHR { $ehr }`
        FindCompositionLine = "find-composition-line",
        /// `Compositions`
        FindCompositions = "find-compositions",
        /// `EHR { $id }`
        FindEhr = "find-ehr",
        /// `Created { $created }`
        FindEhrLine = "find-ehr-line",
        /// `Type an id, a subject, a template or a query name in the search field above.`
        FindEmpty = "find-empty",
        /// `{ $count ->`
        FindMatches = "find-matches",
        /// `No EHR, composition or party has the id { $id }.`
        FindNoIdHolds = "find-no-id-holds",
        /// `Nothing matches “{ $typed }”.`
        FindNothingFor = "find-nothing-for",
        /// `Check the spelling, or open the list of the kind of record you look for.`
        FindNothingLine = "find-nothing-line",
        /// `Nothing found`
        FindNothingTitle = "find-nothing-title",
        /// `Other ways to find a record`
        FindOtherWays = "find-other-ways",
        /// `{ $kind }`
        FindPartyLine = "find-party-line",
        /// `Stored query, version { $version }`
        FindQueryLine = "find-query-line",
        /// `One answer for an id, a subject, a template or a stored query.`
        FindSubtitle = "find-subtitle",
        /// `Template: { $concept }`
        FindTemplateLine = "find-template-line",
        /// `Find`
        FindTitle = "find-title",
        /// `The CDR refused the search over { $kinds }; the answer may be incomplete.`
        FindUnfinishedLine = "find-unfinished-line",
        /// `Part of the search did not finish`
        FindUnfinishedTitle = "find-unfinished-title",
        /// `EHRs`
        FindWayEhr = "find-way-ehr",
        /// `Every EHR, with the subject search.`
        FindWayEhrLine = "find-way-ehr-line",
        /// `Parties`
        FindWayParty = "find-way-party",
        /// `A party by its id or its tags.`
        FindWayPartyLine = "find-way-party-line",
        /// `By subject`
        FindWaySubject = "find-way-subject",
        /// `An EHR by its subject id and namespace.`
        FindWaySubjectLine = "find-way-subject-line",
    }
}
