// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The words of the operations beyond the openEHR API the viewer offers on
//! records and on the repository (archiving, EHR Extracts, dump and load),
//! in the catalogs under `locales/operations/`.

crate::locale::messages! {
    /// A string an operation draws, by its message in its catalog.
    OperationsMsg in "./locales/operations" {
        /// `Archive…`
        ArchiveItem = "archive-item",
        /// `Restore from the archive…`
        RestoreItem = "restore-item",
        /// `Archive this record?`
        ArchiveTitle = "archive-title",
        /// `Restore this record from the archive?`
        RestoreTitle = "restore-title",
        /// `The EHR and all its content move to the cold tier. It stays readable by id, but it leaves every query result until it is restored.`
        ArchiveEhrMessage = "archive-ehr-message",
        /// `The EHR's archived content moves back to the primary tier and appears in query results again.`
        RestoreEhrMessage = "restore-ehr-message",
        /// `The party moves to the cold tier. It stays readable by id, but it leaves every query result until it is restored. Its relationships stay where they are.`
        ArchivePartyMessage = "archive-party-message",
        /// `The party's archived content moves back to the primary tier and appears in query results again.`
        RestorePartyMessage = "restore-party-message",
        /// `Archive`
        ArchiveConfirm = "archive-confirm",
        /// `Restore`
        RestoreConfirm = "restore-confirm",
        /// `Archived`
        Archived = "archived",
        /// `Restored from the archive`
        Restored = "restored",
        /// `The CDR did not archive the record.`
        ArchiveRefused = "archive-refused",
        /// `The CDR did not restore the record.`
        RestoreRefused = "restore-refused",
        /// `Export as EHR Extract`
        ExportExtract = "export-extract",
        /// `Import EHR Extract`
        ImportExtract = "import-extract",
        /// `An EHR Extract, imported as a new EHR with all its content.`
        ImportExtractSubtitle = "import-extract-subtitle",
        /// `EHR Extract`
        ExtractSource = "extract-source",
        /// `Choose a file`
        ExtractChooseFile = "extract-choose-file",
        /// `Paste the EHR Extract, JSON or XML, or choose a file`
        ExtractPlaceholder = "extract-placeholder",
        /// `EHR id (empty: the id the extract carries)`
        ExtractTarget = "extract-target",
        /// `Import`
        ExtractImport = "extract-import",
        /// `EHR imported`
        ExtractImported = "extract-imported",
        /// `The CDR did not import the extract.`
        ExtractImportRefused = "extract-import-refused",
        /// `Dump`
        Dump = "dump",
        /// `Writes an archive of every EHR and every standalone party to the location, on the CDR's own file system.`
        DumpIntro = "dump-intro",
        /// `Content format`
        DumpFormat = "dump-format",
        /// `Compression`
        DumpCompression = "dump-compression",
        /// `Segment size (kb)`
        DumpSegment = "dump-segment",
        /// `The CDR's default`
        DumpDefault = "dump-default",
        /// `The CDR did not dump the repository.`
        DumpRefused = "dump-refused",
        /// `Load`
        Load = "load",
        /// `Load…`
        LoadAsk = "load-ask",
        /// `Loads the archive at the location, on the CDR's own file system, into this repository.`
        LoadIntro = "load-intro",
        /// `Load the archive into this repository?`
        LoadTitle = "load-title",
        /// `Every record the archive holds is written into this repository. An EHR or a party it holds already is reported and skipped. A load is not undone.`
        LoadMessage = "load-message",
        /// `The CDR did not load the archive.`
        LoadRefused = "load-refused",
        /// `Location on the CDR`
        RepositoryLocation = "repository-location",
        /// `Every record moved.`
        RepositoryAllMoved = "repository-all-moved",
        /// `Records not moved`
        RepositoryNotMoved = "repository-not-moved",
        /// `Record`
        RepositoryEntity = "repository-entity",
        /// `Why`
        RepositoryError = "repository-error",
        /// `Binding keys`
        BindingKeys = "binding-keys",
        /// `This person carries no identifier with an issuer, so no login binds to it.`
        NoBindingKey = "no-binding-key",
        /// `Issuer`
        KeyIssuer = "key-issuer",
        /// `Id`
        KeyId = "key-id",
        /// `Re-key…`
        RekeyAsk = "rekey-ask",
        /// `Re-key the binding key under`
        RekeyTitle = "rekey-title",
        /// `The person gets a new version carrying the new key, and the old key stops binding in the same commit. One access record carries both keys.`
        RekeyMessage = "rekey-message",
        /// `New id`
        RekeyNewId = "rekey-new-id",
        /// `Re-key`
        Rekey = "rekey",
        /// `Binding key changed`
        Rekeyed = "rekeyed",
        /// `The CDR did not change the binding key.`
        RekeyRefused = "rekey-refused",
    }
}
