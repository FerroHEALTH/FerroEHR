// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The confidentiality of the data of one EHR, in the order `N` < `R` < `V`.
//!
//! A datum's level is the highest of the EHR's marking, the marking of a
//! label that targets it, and the confidentiality each classification whose
//! scope it falls in gives it (draft access control, The Decision Procedure,
//! Terms). Which items a level covers is our own design over that text
//! (#6057).

use std::collections::BTreeMap;

use openehr_rm::draft::access_control::access_control::access_label::AccessLabel;
use openehr_rm::draft::access_control::terminology::{Confidentiality, ForeignCode};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use super::{Held, Polarity, Pred, Scopes, labelled};

impl Scopes {
    /// The items whose confidentiality is `level` or higher: the whole EHR
    /// when its marking is, the objects a label marks so, and the data a
    /// classification gives such a level, an unreadable label read by
    /// `polarity`. A classification's own scope reads no classification, so
    /// classifications never refer to each other.
    #[must_use]
    pub fn at_least(&self, level: Confidentiality, polarity: Polarity) -> Pred {
        self.at_least_of(level, polarity, true)
    }

    /// [`Self::at_least`], with the classifications when `classified`.
    pub(super) fn at_least_of(
        &self,
        level: Confidentiality,
        polarity: Polarity,
        classified: bool,
    ) -> Pred {
        let mut preds = vec![Pred::of(self.marking >= level)];
        preds.extend(
            self.confidential
                .range(level..)
                .map(|(_, objects)| labelled(objects, polarity)),
        );
        if classified {
            preds.extend(
                self.classifications
                    .iter()
                    .filter(|(_, _, given)| *given >= level)
                    .map(|(pred, _, _)| pred.clone()),
            );
        }
        Pred::any(preds)
    }

    /// The levels above `N` some datum of the EHR may carry.
    #[must_use]
    pub fn levels(&self) -> Vec<Confidentiality> {
        Confidentiality::ALL
            .into_iter()
            .filter(|level| *level > Confidentiality::Normal)
            .filter(|level| self.at_least(*level, Polarity::Deny) != Pred::False)
            .collect()
    }

    /// The items a data scope naming `levels` covers: those whose
    /// confidentiality is the highest of them or lower, every item when one
    /// is `V`; `classified` reads the classifications, which a
    /// classification's own scope does not.
    pub(super) fn at_most(
        &self,
        levels: &[DvCodedText],
        polarity: Polarity,
        classified: bool,
    ) -> Pred {
        let highest = levels
            .iter()
            .map(level_of)
            .max()
            .unwrap_or(Confidentiality::Normal);
        match Confidentiality::ALL
            .into_iter()
            .find(|above| *above > highest)
        {
            Some(above) => Pred::negate(self.at_least_of(above, polarity.flipped(), classified)),
            None => Pred::True,
        }
    }
}

/// The objects labelled with each sensitivity and with each confidentiality
/// above `N`.
pub(super) type Labelled = (
    BTreeMap<ForeignCode, Held<Vec<Uuid>>>,
    BTreeMap<Confidentiality, Held<Vec<Uuid>>>,
);

/// The level `coded` names; the write check holds it to the code system, so
/// one that does not read is read as the highest.
pub(super) fn level_of(coded: &DvCodedText) -> Confidentiality {
    Confidentiality::read(coded).unwrap_or(Confidentiality::VeryRestricted)
}

/// The confidentiality the marking of `label` gives its object; `N` with
/// none.
#[must_use]
pub fn label_level(label: &AccessLabel) -> Confidentiality {
    label
        .confidentiality
        .as_ref()
        .map_or(Confidentiality::Normal, |marking| {
            level_of(&marking.confidentiality)
        })
}
