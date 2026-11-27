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
