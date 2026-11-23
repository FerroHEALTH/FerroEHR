// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The publication of the access log's checkpoints outside the database
//! (#5704).
//!
//! A copy of the checkpoints kept where a database writer cannot write shows a
//! removal of the newest entries with the checkpoints that seal them: the log
//! does not reach the newest published checkpoint. One run per [`INTERVAL`]
//! writes the newest checkpoint, when it is newer than the last one published,
//! as its own file, created once and never rewritten, so the directory can sit
//! on write-once storage. Replicas publish the same checkpoints under the same
//! names.
//!
//! No openEHR spec governs log publication: our own design
//! (`docs/specifications/access-log`, semantics §Integrity).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use jiff::Timestamp;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
use tokio::io::AsyncWriteExt as _;

use crate::access_log::checkpoint::Checkpoints;
use crate::access_log::store::AccessLogError;

/// The time between two publication runs: thirty seconds.
// NOTE: our own design (#5704); the newest entries a run has not published are the ones a removal hides.
pub const INTERVAL: Duration = Duration::from_secs(30);

/// Why a checkpoint could not be published.
#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    /// The checkpoint could not be read from the store, or marked published
    /// there.
    #[error("the newest access-log checkpoint could not be read or marked published")]
    Read(#[source] AccessLogError),
    /// The checkpoint has no JSON form.
    #[error("the access-log checkpoint does not serialise")]
    Serialise(#[source] serde_json::Error),
    /// The file could not be written.
    #[error("the access-log checkpoint could not be written to {path}")]
    Write {
        /// The file.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },
    /// A file of the same name holds another checkpoint.
    #[error("{path} already holds another checkpoint at the same sequence")]
    Conflict {
        /// The file.
        path: PathBuf,
    },
}

/// The outcome of the last publication run, read by the health indicator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublicationState {
    /// No run has finished yet.
    Pending,
    /// The newest checkpoint at the run is published.
    Published {
        /// Its sequence; `None` while the log holds no checkpoint.
        sequence: Option<i64>,
        /// When the run finished.
        at: Timestamp,
    },
    /// The run could not publish.
    Failed {
        /// When the run finished.
        at: Timestamp,
        /// What failed.
        reason: String,
    },
}

/// The publication of this system's checkpoints: where they go and how the
/// last run went.
#[derive(Debug, Clone)]
pub struct Publication {
    directory: Option<PathBuf>,
    state: Arc<Mutex<PublicationState>>,
    published: Arc<Mutex<Option<i64>>>,
}

impl Publication {
    /// The publication into `directory`; `None` publishes nothing.
    #[must_use]
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            state: Arc::new(Mutex::new(PublicationState::Pending)),
            published: Arc::new(Mutex::new(None)),
        }
    }

    /// The directory checkpoints are published to, when one is configured.
    #[must_use]
    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref()
    }

    /// The outcome of the last run.
    #[must_use]
    pub fn state(&self) -> PublicationState {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Publish the newest checkpoint of `checkpoints` when it is newer than
    /// the last one this process published; its sequence, or `None` while
    /// the log holds no checkpoint or no directory is configured.
    ///
    /// # Errors
    /// [`PublishError`] when the checkpoint cannot be read or written, or a
    /// file of its name holds another checkpoint.
    pub async fn publish(&self, checkpoints: &Checkpoints) -> Result<Option<i64>, PublishError> {
        let Some(directory) = &self.directory else {
            return Ok(None);
        };
        let newest = checkpoints.latest(None).await.map_err(PublishError::Read)?;
        let Some(newest) = newest else {
            return Ok(None);
        };
        let already = *self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if already.is_some_and(|sequence| sequence >= newest.sequence) {
            return Ok(Some(newest.sequence));
        }
        write_once(directory, &newest).await?;
        checkpoints
            .mark_published(newest.sequence)
            .await
            .map_err(PublishError::Read)?;
        *self
            .published
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(newest.sequence);
        Ok(Some(newest.sequence))
    }

    /// Run [`Self::publish`] and keep its outcome for the health indicator.
    pub async fn run(&self, checkpoints: &Checkpoints) {
        let outcome = self.publish(checkpoints).await;
        let at = Timestamp::now();
        let state = match outcome {
            Ok(sequence) => {
                tracing::debug!(?sequence, "access-log checkpoint published");
                PublicationState::Published { sequence, at }
            }
            Err(error) => {
                tracing::error!(error = %error, "the access-log checkpoint could not be published");
                PublicationState::Failed {
                    at,
                    reason: error.to_string(),
                }
            }
        };
        *self.state.lock().unwrap_or_else(PoisonError::into_inner) = state;
    }
}

/// The file `checkpoint` is published as in `directory`: one name per
/// sequence, which sorts in sequence order.
#[must_use]
pub fn file_name(directory: &Path, checkpoint: &AccessLogCheckpoint) -> PathBuf {
    directory.join(format!(
        "access-log-checkpoint-{:020}.json",
        checkpoint.sequence
    ))
}

/// Write `checkpoint` into `directory` as a new file; a file of its name
/// that already holds the same checkpoint is the same publication.
async fn write_once(
    directory: &Path,
    checkpoint: &AccessLogCheckpoint,
) -> Result<(), PublishError> {
    let path = file_name(directory, checkpoint);
    let mut text = serde_json::to_string_pretty(checkpoint).map_err(PublishError::Serialise)?;
    text.push('\n');
    let failed = |source| PublishError::Write {
        path: path.clone(),
        source,
    };
    let opened = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .await;
    match opened {
        Ok(mut file) => {
            file.write_all(text.as_bytes()).await.map_err(failed)?;
            file.sync_all().await.map_err(failed)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let held = tokio::fs::read_to_string(&path).await.map_err(failed)?;
            // NOTE: our own design; a file that does not read as a checkpoint holds another one.
            let same = serde_json::from_str::<AccessLogCheckpoint>(&held)
                .is_ok_and(|held| held == *checkpoint);
            if same {
                Ok(())
            } else {
                Err(PublishError::Conflict { path })
            }
        }
        Err(error) => Err(failed(error)),
    }
}

/// One publication run per `interval`, the first at once.
#[expect(
    clippy::infinite_loop,
    reason = "the publication is a detached background task with no shutdown channel; it ends \
              when the runtime drops the task, as the seal schedule does"
)]
pub async fn scheduled(publication: Publication, checkpoints: Checkpoints, interval: Duration) {
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        publication.run(&checkpoints).await;
    }
}
