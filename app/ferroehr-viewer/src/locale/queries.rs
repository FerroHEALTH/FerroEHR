// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the queries group, in the catalogs under `locales/queries/`.
//!
//! Only code of the queries group looks them up, so their catalog travels
//! with that code: the browser loads it with the group's chunks, never with
//! the entry WebAssembly (`crate::locale`).

crate::locale::messages! {
    /// A string the queries group draws, by its message in its catalog.
    QueriesMsg in "./locales/queries" {
        ///  DESC
        Desc = "desc",
        ///  ASC
        Asc = "asc",
        /// `did not parse as a data path`
        DidNotParseDataPath = "did-not-parse-data-path",
        /// `Query saved`
        QuerySaved = "query-saved",
        /// `{ $name } { $version }`
        QueriesAqlTitle = "queries-aql-title",
        /// `Definition`
        QueriesDefinition = "queries-definition",
        /// `Delete version { $version }`
        QueriesDeleteConfirm = "queries-delete-confirm",
        /// `Removes this version from the CDR's stored queries for good, for every client; its other v`
        QueriesDeleteMessage = "queries-delete-message",
        /// `Delete version { $version }…`
        QueriesDeleteNamed = "queries-delete-named",
        /// `The CDR refused to delete the version.`
        QueriesDeleteRefused = "queries-delete-refused",
        /// `Delete { $name } version { $version }?`
        QueriesDeleteTitle = "queries-delete-title",
        /// `Delete this version…`
        QueriesDeleteVersion = "queries-delete-version",
        /// `Stored query version deleted`
        QueriesDeleted = "queries-deleted",
        /// `Nothing was sent: fix the version first.`
        QueriesFixVersion = "queries-fix-version",
        /// `{ $names ->`
        QueriesGroupCount = "queries-group-count",
        /// `{ $name } · version { $version }`
        QueriesLine = "queries-line",
        /// `More actions for the stored query`
        QueriesMore = "queries-more",
        /// `Copy name`
        QueriesCopyName = "queries-copy-name",
        /// `Namespace { $value }`
        QueriesNamespaceChip = "queries-namespace-chip",
        /// `older version`
        QueriesOlderVersion = "queries-older-version",
        /// `Open in Query`
        QueriesOpenInQuery = "queries-open-in-query",
        /// `Numbers and true or false are sent as they are; quote a number to send it as text.`
        QueriesParamHelp = "queries-param-help",
        /// `Read { $version }`
        QueriesReadVersion = "queries-read-version",
        /// `You are reading version { $version }; Latest runs { $latest }.`
        QueriesReadingOlder = "queries-reading-older",
        /// `The stored queries could not be read.`
        QueriesRefused = "queries-refused",
        /// `Saved`
        QueriesSaved = "queries-saved",
        /// `The stored query could not be read.`
        QueriesStoredRefused = "queries-stored-refused",
        /// `The CDR's stored queries, grouped by the namespace of their name.`
        QueriesSubtitle = "queries-subtitle",
        /// `Type`
        QueriesType = "queries-type",
        /// `Unqualified`
        QueriesUnqualified = "queries-unqualified",
        /// `Each version is its own immutable definition; Latest runs the highest.`
        QueriesVersionsIntro = "queries-versions-intro",
        /// `Views of this stored query`
        QueriesViews = "queries-views",
        /// `Which version runs`
        QueriesWhichVersion = "queries-which-version",
        /// `Add a data point`
        QueryAddDataPoint = "query-add-data-point",
        /// `Add a parameter`
        QueryAddParam = "query-add-param",
        /// `AQL`
        QueryAql = "query-aql",
        /// `Query · AQL`
        QueryAqlTitle = "query-aql-title",
        /// `The row limit is not a whole number.`
        QueryBadLimit = "query-bad-limit",
        /// `Build`
        QueryBuild = "query-build",
        /// `The AQL Build made, as it runs; edits here leave Build behind.`
        QueryBuildMade = "query-build-made",
        /// `Built AQL`
        QueryBuiltAql = "query-built-aql",
        /// `This AQL cannot be opened in Build: { $reason }`
        QueryCannotBuild = "query-cannot-build",
        /// `The template's data points could not be read.`
        QueryCatalogRefused = "query-catalog-refused",
        /// `Column name`
        QueryColumnName = "query-column-name",
        /// `Column name of data point { $number }`
        QueryColumnNameN = "query-column-name-n",
        /// `Conditions`
        QueryConditions = "query-conditions",
        /// `Data point`
        QueryDataPoint = "query-data-point",
        /// `Data point { $number }`
        QueryDataPointN = "query-data-point-n",
        /// `Data points`
        QueryDataPoints = "query-data-points",
        /// `Direction`
        QueryDirection = "query-direction",
        /// `The query ends before it is complete.`
        QueryEndedEarly = "query-ended-early",
        /// `{ $count ->`
        QueryGrammarHeading = "query-grammar-heading",
        /// `Nothing was sent: the grammar check found an error.`
        QueryGrammarStops = "query-grammar-stops",
        /// `The AQL it builds`
        QueryItBuilds = "query-it-builds",
        /// `Row limit`
        QueryLimit = "query-limit",
        /// `Namespace`
        QueryNamespace = "query-namespace",
        /// `Newest first`
        QueryNewestFirst = "query-newest-first",
        /// `No data point yet; the query returns nothing until you add one.`
        QueryNoDataPoints = "query-no-data-points",
        /// `No order`
        QueryNoOrder = "query-no-order",
        /// `No parameter.`
        QueryNoParams = "query-no-params",
        /// `The builder cannot write this query yet: { $reason }`
        QueryNotBuilt = "query-not-built",
        /// `The value of { $path }, { $value }, is not a number.`
        QueryNotNumber = "query-not-number",
        /// `Nothing to run: write a query first.`
        QueryNothingToRun = "query-nothing-to-run",
        /// `Oldest first`
        QueryOldestFirst = "query-oldest-first",
        /// `Open in AQL`
        QueryOpenInAql = "query-open-in-aql",
        /// `Open in Build`
        QueryOpenInBuild = "query-open-in-build",
        /// `Open it`
        QueryOpenStored = "query-open-stored",
        /// `Opened from the stored query { $name } { $version }.`
        QueryOpenedStored = "query-opened-stored",
        /// `Order by`
        QueryOrderBy = "query-order-by",
        /// `Order and limit`
        QueryOrderLimit = "query-order-limit",
        /// `Name of parameter { $number }`
        QueryParamNameN = "query-param-name-n",
        /// `Value of parameter { $number }`
        QueryParamValueN = "query-param-value-n",
        /// `Parameters`
        QueryParameters = "query-parameters",
        /// `Remove the data point { $name }`
        QueryRemoveDataPoint = "query-remove-data-point",
        /// `Remove the parameter { $name }`
        QueryRemoveParam = "query-remove-param",
        /// `Returns`
        QueryReturns = "query-returns",
        /// `Run`
        QueryRun = "query-run",
        /// `The CDR refused the query.`
        QueryRunRefused = "query-run-refused",
        /// `Save to the CDR`
        QuerySave = "query-save",
        /// `Save`
        QuerySaveAction = "query-save-action",
        /// `The CDR refused to store the query.`
        QuerySaveRefused = "query-save-refused",
        /// `Whole compositions`
        QueryShapeCompositions = "query-shape-compositions",
        /// `A count`
        QueryShapeCount = "query-shape-count",
        /// `Data points`
        QueryShapeData = "query-shape-data",
        /// `EHRs (cohort)`
        QueryShapeEhrs = "query-shape-ehrs",
        /// `The stored query could not be read.`
        QueryStoredRefused = "query-stored-refused",
        /// `Build AQL from a template, or write it; run it against the CDR.`
        QuerySubtitle = "query-subtitle",
        /// `The AQL`
        QueryTheAql = "query-the-aql",
        /// `Unexpected { $token } here.`
        QueryUnexpected = "query-unexpected",
        /// `The template has no data point at { $path }.`
        QueryUnknownPath = "query-unknown-path",
        /// `Empty writes the version the CDR assigns.`
        QueryVersionHelp = "query-version-help",
        /// `This version of this name already exists, and a saved version cannot change.`
        QueryVersionTaken = "query-version-taken",
        /// `Ways to write the query`
        QueryWays = "query-ways",
        /// `The x axis is { $axis }, at its true spacing.`
        ResultsAxis = "results-axis",
        /// `The numeric columns over the results`
        ResultsChartLabel = "results-chart-label",
        /// `{ $series } over { $axis }.`
        ResultsChartSummary = "results-chart-summary",
        /// `Run the query to see its rows here.`
        ResultsNotRun = "results-not-run",
        /// `Results`
        ResultsTitle = "results-title",
        /// `{ $count ->`
        ResultsWindow = "results-window",
        /// `{ $count ->`
        ResultsWindowOwn = "results-window-own",
        /// `Name the query to save it.`
        QuerySaveNameMissing = "query-save-name-missing",
        /// `Sends`
        QueriesSends = "queries-sends",
        /// `Latest sends no version; Prefix and Exact send the one typed here.`
        QueriesVersionHelp = "queries-version-help",
    }
}
