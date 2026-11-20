// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Where the service hands `DV_MULTIMEDIA` content to the blob store: the
//! externalization every write runs before it holds a database connection, and
//! the expansion a read runs on request. No openEHR spec governs storage
//! movement; the stored and served canonical forms are RM `DV_MULTIMEDIA`
//! either way. Our own design (#5934).
//!
//! The upload runs first and the commit after it, so no pooled connection and
//! no transaction waits on the blob store. A commit that fails after its upload
//! leaves an unreferenced blob, which costs storage and never correctness; the
//! commit transaction refuses a body that still holds a value to externalize,
//! so a write path that skips this fails loudly instead of uploading under a
//! lock.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): multimedia offload over stored fragments \
              (family 8): the body is the canonical JSON the commit stores"
)]

use serde_json::Value;

use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::versioning::body::Body;
use crate::versioning::change::Change;

impl FerroEhrService {
    /// Externalizes every large inline `DV_MULTIMEDIA` in `body`, uploading the
    /// blobs, before the write that stores it takes a connection.
    ///
    /// # Errors
    /// [`ServiceError`] when an inline value is malformed or an upload fails;
    /// the write is then refused and nothing is committed.
    #[cfg(feature = "multimedia")]
    pub(crate) async fn externalize(&self, body: &mut Value) -> Result<(), ServiceError> {
        let Some(engine) = &self.multimedia else {
            return Ok(());
        };
        // NOTE: our own design (#5774), an upload waits on the blob store, so the request gives
        // back the connections it keeps unless the offload is done at once (no value to upload).
        crate::db::acquire::awaiting(engine.offload(body))
            .await
            .map_err(|e| ServiceError::internal("externalize multimedia", e))
    }

    /// The slim twin: externalization is compiled out, so a body is stored as
    /// it came.
    ///
    /// # Errors
    /// Infallible in this configuration.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "signature parity with the multimedia twin, which awaits"
    )]
    pub(crate) async fn externalize(&self, _body: &mut Value) -> Result<(), ServiceError> {
        Ok(())
    }

    /// [`Self::externalize`] over the content a change carries; a delete
    /// carries none.
    ///
    /// # Errors
    /// The [`Self::externalize`] failure.
    pub(crate) async fn externalize_change(&self, change: &mut Change) -> Result<(), ServiceError> {
        match change {
            Change::Create { body, .. } | Change::Modify { body, .. } => {
                self.externalize_body(body).await
            }
            Change::Delete { .. } => Ok(()),
        }
    }

    /// [`Self::externalize`] over a commit body: a typed COMPOSITION is
    /// offloaded in place, with no canonical value built.
    ///
    /// # Errors
    /// The offload's failure.
    pub(crate) async fn externalize_body(&self, body: &mut Body) -> Result<(), ServiceError> {
        match body {
            Body::Canonical(canonical) => self.externalize(canonical).await,
            typed => match typed.as_rm_mut() {
                Some(rm) => self.externalize_typed(rm).await,
                None => Ok(()),
            },
        }
    }

    /// Externalizes every large inline `DV_MULTIMEDIA` of the typed
    /// RM value `rm` in place, uploading the blobs, before the write that
    /// stores it takes a connection.
    ///
    /// # Errors
    /// [`ServiceError`] when an inline value is malformed or an upload fails.
    #[cfg(feature = "multimedia")]
    pub(crate) async fn externalize_typed(
        &self,
        rm: &mut dyn openehr_rm::v1_2::access::RmObject,
    ) -> Result<(), ServiceError> {
        let Some(engine) = &self.multimedia else {
            return Ok(());
        };
        // NOTE: our own design (#5774), as `externalize`: an upload keeps no connection.
        crate::db::acquire::awaiting(engine.offload_typed(rm))
            .await
            .map_err(|e| ServiceError::internal("externalize multimedia", e))
    }

    /// The slim twin: externalization is compiled out.
    ///
    /// # Errors
    /// Infallible in this configuration.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "signature parity with the multimedia twin, which awaits"
    )]
    pub(crate) async fn externalize_typed(
        &self,
        _rm: &mut dyn openehr_rm::v1_2::access::RmObject,
    ) -> Result<(), ServiceError> {
        Ok(())
    }
}

impl FerroEhrService {
    /// Re-inline externalized multimedia content into a canonical body before
    /// it is served (the S3/object-store externalization extension — no
    /// openEHR spec governs storage movement; the served canonical form is
    /// unchanged). With no engine configured the stored form is served
    /// unchanged.
    ///
    /// # Errors
    /// [`ServiceError`] when the configured multimedia engine fails to expand a
    /// reference (e.g. the external object store is unreachable).
    #[cfg(feature = "multimedia")]
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, ServiceError> {
        let Some(engine) = &self.multimedia else {
            // With no store reachable, serving the stored form is correct only
            // when there is nothing to expand: answering 200 with a compact
            // reference would silently withhold clinical content this server
            // externalized.
            if ferroehr_ext::multimedia::references_external_blob(&body) {
                return Err(ServiceError::exception(
                    "expand a multimedia reference: this record references externalized \
                     multimedia but no object store is configured, so the content cannot be \
                     re-inlined (configure record.multimedia.s3.url, or re-enable multimedia, to \
                     serve it)",
                ));
            }
            return Ok(body);
        };
        let mut body = body;
        engine
            .expand(&mut body)
            .await
            .map_err(|e| ServiceError::internal("expand a multimedia reference", e))?;
        Ok(body)
    }

    /// Serves the stored canonical form unchanged: externalization is compiled
    /// out of this build, so nothing was ever moved out of a body.
    ///
    /// # Errors
    /// Infallible in this configuration; the `Result` mirrors the multimedia
    /// twin so callers are configuration-independent.
    ///
    /// The `async` is the twin's signature, not this body's need: the one
    /// caller lives in `ferroehr-rest` and awaits this method however the
    /// binary was built.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "signature parity with the multimedia twin, which awaits"
    )]
    pub async fn expand_multimedia(&self, body: Value) -> Result<Value, ServiceError> {
        Ok(body)
    }
}
