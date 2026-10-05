// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! List handling — the SM cursor convention (`master02-overview.adoc`
//! §List Handling).

/// The SM list-cursor parameters, used by every unbounded-list call.
///
/// `master02-overview.adoc` §List Handling: "Calls that produce a container
/// result potentially containing unlimited numbers of elements can be managed
/// in a typical 'DB cursor' fashion".
///
/// `item_offset`: "Optional parameter specifying offset in query result items
/// … starting at zero. … Zero signifies that items starting from the first
/// item should be returned."
/// `items_to_fetch`: "Optional parameter specifying number of result items to
/// fetch, starting from the item indicated by `item_offset`. A zero means
/// 'all'."
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Page {
    /// Offset in result items at which to start returning items (0-based).
    pub item_offset: Option<u64>,
    /// Number of result items to fetch from `item_offset`; 0 (or `None`) =
    /// all.
    pub items_to_fetch: Option<u64>,
}

impl Page {
    /// The whole list — no offset, no limit.
    #[must_use]
    pub fn all() -> Self {
        Self::default()
    }

    /// The effective 0-based offset (`item_offset`, defaulting to 0).
    #[must_use]
    pub fn offset(self) -> u64 {
        self.item_offset.unwrap_or(0)
    }

    /// The effective fetch limit: `None` means all (a `Some(0)` in the SM
    /// also means 'all', normalized here).
    #[must_use]
    pub fn limit(self) -> Option<u64> {
        match self.items_to_fetch {
            None | Some(0) => None,
            some => some,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_normalizes_zero_fetch_to_all() {
        // master02 §List Handling: "A zero means 'all'".
        let page = Page {
            item_offset: Some(3),
            items_to_fetch: Some(0),
        };
        assert_eq!(page.offset(), 3);
        assert_eq!(page.limit(), None);
    }
}
