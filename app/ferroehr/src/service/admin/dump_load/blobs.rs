// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The externalized `DV_MULTIMEDIA` blobs a dump carries as `blobs/<key>`
//! entries and a load puts back. No openEHR spec governs the archive or
//! multimedia offload: our own design.
//!
//! An export streams each blob from the store into its entry, proving the
//! bytes against their key on the way. A load writes each entry back through
//! the store, which refuses an entry whose bytes are not the ones its name
//! says, so a tampered archive cannot plant content under a key.

use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use ferroehr_ext::multimedia::MultimediaError;
use ferroehr_ext::multimedia::blob::{BlobKey, NotABlobKey};
use sqlx::PgPool;

use super::{
    ArchiveReader, ArchiveWriter, BLOB_PREFIX, DemographicRecord, EhrRecord, archive_refusal,
    file_not_writable, sevenz_fault, zip_fault,
};
use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::storage::dump_repo;
use crate::storage::dump_repo::VersionRecord;

/// How many bytes of an archived blob go to the store per write.
const IMPORT_CHUNK: usize = 1024 * 1024;

/// One archive entry being written chunk by chunk.
enum EntrySink<'a> {
    /// A loose file under `file_sys_loc`.
    File {
        path: PathBuf,
        file: BufWriter<File>,
    },
    /// An entry of the zip container, started and written in place.
    Zip {
        path: &'a Path,
        zip: &'a mut zip::ZipWriter<File>,
    },
    /// An entry of the 7z container, which takes a whole entry at once.
    SevenZip {
        path: &'a Path,
        name: String,
        writer: &'a mut sevenz_rust2::ArchiveWriter<File>,
        // NOTE: sevenz_rust2 `push_archive_entry` reads one complete source per entry, so the 7z form holds one blob.
        buffer: Vec<u8>,
    },
}

impl ArchiveWriter {
    /// Starts the entry `name`, to be filled by [`EntrySink::write`] and
    /// closed by [`EntrySink::finish`].
    fn entry(&mut self, name: &str) -> Result<EntrySink<'_>, ServiceError> {
        match self {
            Self::Directory { dir } => {
                let path = dir.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| file_not_writable(parent, &e))?;
                }
                let file = File::create(&path).map_err(|e| file_not_writable(&path, &e))?;
                Ok(EntrySink::File {
                    path,
                    file: BufWriter::new(file),
                })
            }
            Self::Zip { path, zip } => {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                zip.start_file(name, options)
                    .map_err(|e| zip_fault(path, "start entry", &e))?;
                Ok(EntrySink::Zip { path, zip })
            }
            Self::SevenZip { path, writer } => Ok(EntrySink::SevenZip {
                path,
                name: name.to_owned(),
                writer,
                buffer: Vec::new(),
            }),
        }
    }
}

impl EntrySink<'_> {
    /// Appends `chunk` to the entry.
    fn write(&mut self, chunk: &[u8]) -> Result<(), ServiceError> {
        match self {
            Self::File { path, file } => file
                .write_all(chunk)
                .map_err(|e| file_not_writable(path, &e)),
            Self::Zip { path, zip } => zip
                .write_all(chunk)
                .map_err(|e| file_not_writable(path, &e)),
            Self::SevenZip { buffer, .. } => {
                buffer.extend_from_slice(chunk);
                Ok(())
            }
        }
    }

    /// Completes the entry.
    fn finish(self) -> Result<(), ServiceError> {
        match self {
            Self::File { path, mut file } => file.flush().map_err(|e| file_not_writable(&path, &e)),
            Self::Zip { .. } => Ok(()),
            Self::SevenZip {
                path,
                name,
                writer,
                buffer,
            } => writer
                .push_archive_entry(
                    sevenz_rust2::ArchiveEntry::new_file(&name),
                    Some(buffer.as_slice()),
                )
                .map(|_| ())
                .map_err(|e| sevenz_fault(path, &format!("write entry {name}"), &e)),
        }
    }
}

/// Why streaming one blob into the archive stopped.
enum ExportFault {
    /// The store failed, or the bytes did not prove against their key.
    Blob(MultimediaError),
    /// The archive entry could not be written.
    Archive(ServiceError),
}

impl From<MultimediaError> for ExportFault {
    fn from(e: MultimediaError) -> Self {
        Self::Blob(e)
    }
}

impl FerroEhrService {
    /// Streams every externalized `DV_MULTIMEDIA` blob the exported records
    /// reference into a `blobs/<key>` archive entry, returning the keys
    /// written (empty when no store is configured).
    ///
    /// The keys come from `blob_ref`, the reference index the node write path
    /// maintains in the same transaction as the nodes it describes, so the
    /// export reads one index per domain instead of walking every exported
    /// body in process. An exported version that references no blob contributes
    /// no row, and an archived version's references moved to the cold tier with
    /// it, so both tiers answer from the same relation.
    ///
    /// # Errors
    /// [`ServiceError`] when a reference read, a blob read or proof, or an
    /// archive write fails.
    pub(super) async fn export_referenced_blobs(
        &self,
        archive: &mut ArchiveWriter,
        records: &[EhrRecord],
        demographic: &[DemographicRecord],
    ) -> Result<Vec<String>, ServiceError> {
        let Some(engine) = &self.multimedia else {
            return Ok(Vec::new());
        };
        let mut uris = self
            .blob_uris_of_versions(&self.pool, records.iter().flat_map(|r| r.versions.iter()))
            .await?;
        uris.extend(
            self.blob_uris_of_versions(
                &self.demographic_pool,
                demographic.iter().flat_map(|r| r.versions.iter()),
            )
            .await?,
        );
        // NOTE: a URI this store did not mint names someone else's bytes, which
        // is an absent key rather than a defect — the index records every URI a
        // stored body carries, foreign ones included.
        let mut keys: Vec<BlobKey> = uris
            .iter()
            .filter_map(|uri| engine.store().key_from_uri(uri))
            .collect();
        keys.sort_unstable();
        keys.dedup();
        for key in &keys {
            let mut entry = archive.entry(&format!("{BLOB_PREFIX}{key}"))?;
            let streamed = engine
                .store()
                .read_verified(key, |chunk| {
                    entry.write(chunk).map_err(ExportFault::Archive)
                })
                .await;
            match streamed {
                Ok(_) => entry.finish()?,
                Err(ExportFault::Archive(e)) => return Err(e),
                Err(ExportFault::Blob(e)) => {
                    return Err(ServiceError::internal("export a multimedia blob", e));
                }
            }
        }
        Ok(keys.iter().map(BlobKey::to_string).collect())
    }

    /// The blob URIs `blob_ref` records for the given versions, in one domain.
    ///
    /// Keyed on `(vo_id, sys_version)` without a tier predicate, so a version
    /// the export read from the cold partition answers from the reference rows
    /// its foreign key carried there. Our own extension.
    ///
    /// # Errors
    /// [`ServiceError::Database`] when the read fails.
    async fn blob_uris_of_versions<'a>(
        &self,
        pool: &PgPool,
        versions: impl Iterator<Item = &'a VersionRecord>,
    ) -> Result<Vec<String>, ServiceError> {
        let (vo_ids, sys_versions): (Vec<VoId>, Vec<i32>) =
            versions.map(|v| (v.vo_id, v.sys_version)).unzip();
        if vo_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(dump_repo::read::version_blob_uris(pool, &vo_ids, &sys_versions).await?)
    }

    /// Writes each archived blob (`blobs/<key>`) back into the store on load,
    /// each proven against its key as it goes (idempotent, content-addressed).
    /// A no-op when the archive carries no blobs.
    ///
    /// # Errors
    /// The archive refusal for a manifest name that is not a blob key or an
    /// entry whose bytes are not the ones its key names;
    /// [`ServiceError::precondition`] when the archive carries blobs and no
    /// store is configured; [`ServiceError`] when the store fails.
    pub(super) async fn import_blobs(
        &self,
        archive: &mut ArchiveReader,
        dir: &Path,
        blobs: &[String],
    ) -> Result<(), ServiceError> {
        if blobs.is_empty() {
            return Ok(());
        }
        let Some(engine) = &self.multimedia else {
            // The archive carries blobs but this target has no store configured.
            return Err(ServiceError::precondition(
                "archive carries externalized multimedia blobs but multimedia \
                 externalization is not enabled on this server",
            ));
        };
        for name in blobs {
            let entry = format!("{BLOB_PREFIX}{name}");
            let key: BlobKey = name
                .parse()
                .map_err(|e: NotABlobKey| archive_refusal(dir, &entry, e))?;
            let bytes = Bytes::from(archive.read(&entry)?);
            let size = u64::try_from(bytes.len()).map_err(|e| archive_refusal(dir, &entry, e))?;
            let chunks = (0..bytes.len()).step_by(IMPORT_CHUNK).map(|start: usize| {
                Ok(bytes.slice(start..bytes.len().min(start.saturating_add(IMPORT_CHUNK))))
            });
            match engine.store().write_if_absent(&key, size, chunks).await {
                Ok(()) => {}
                Err(e @ (MultimediaError::Integrity { .. } | MultimediaError::Size { .. })) => {
                    return Err(archive_refusal(dir, &entry, e));
                }
                Err(e) => return Err(ServiceError::internal("import a multimedia blob", e)),
            }
        }
        Ok(())
    }
}
