// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The objects a request's own writes committed, whose answer is built from
//! the version that request committed, never judged as a read of stored
//! data. Scoped to the objects and the request: every later read is judged.
//! No openEHR spec governs this: our own design (owner ruling on #5621).

use std::sync::PoisonError;

use uuid::Uuid;

use crate::access_eval::condition::RequestAccess;

impl RequestAccess {
    /// Notes that this request committed a version of the object `vo_id`,
    /// which its response is built from.
    pub fn wrote(&self, vo_id: Uuid) {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(vo_id);
    }

    /// Whether this request committed a version of the object `vo_id`.
    pub(super) fn written(&self, vo_id: Uuid) -> bool {
        self.written
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&vo_id)
    }
}
