// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access gates of one query, read once when the builder starts: the EHDS
//! Art. 8 restriction of access by professionals and the Art. 3(3) delay of
//! the person's own access (`docs/law/eu/ehds/text.html`), each published for
//! the request by `crate::system_log::access_context`. No openEHR spec governs
//! either: our own design/extension.

use sea_query::{Expr, ExprTrait as _};

use super::expr::col;
use crate::db::iden::VoHead;

/// Which access gates hold for the query being built.
#[derive(Debug, Clone, Copy)]
pub(super) struct Gates {
    /// The Art. 8 restriction: a restricted object or EHR contributes no row.
    pub(super) access_restricted: bool,
    /// The Art. 3(3) delay: an object withheld from the person contributes no
    /// row.
    person_withheld: bool,
}

impl Gates {
    /// The gates the current request publishes.
    pub(super) fn current() -> Self {
        Self {
            access_restricted: crate::system_log::access_context::access_restriction_enforced(),
            person_withheld: crate::system_log::access_context::access_delay_enforced(),
        }
    }

    /// The predicates on the head row `h` of one versioned object.
    pub(super) fn on_head(self, h: &str) -> Vec<Expr> {
        let mut predicates = Vec::new();
        // NOTE: EHDS Art. 8, an object under an access restriction contributes no row.
        if self.access_restricted {
            predicates.push(col(h, VoHead::AccessRestrictedAt).is_null());
        }
        // NOTE: EHDS Art. 3(3), an object withheld from the person contributes no row.
        if self.person_withheld {
            let until = col(h, VoHead::PersonWithheldUntil);
            predicates.push(until.clone().is_null().or(until.lte(Expr::cust("now()"))));
        }
        predicates
    }
}
