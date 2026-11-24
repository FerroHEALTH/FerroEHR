// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which data the caller's care relationships cover (draft access control,
//! decision procedure, Holding a Care Relationship).
//!
//! A relationship covers a datum when it names no `episode`, the datum is
//! unplaced, one of the datum's episodes is its `episode`, or a scope of
//! `ACCESS_POLICY.episode_shared` names the datum; for a caller of kind
//! `access_service` it also covers the data of an EHDS Art. 14(1) priority
//! category, which Art. 11(3) makes reachable across healthcare tasks. The
//! rule is the draft's own: openEHR does not scope access per episode.

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::terminology::CallerKind;

use crate::access_eval::care_tie::{CareNeed, Tie};
use crate::access_eval::procedure::Judged;
use crate::access_eval::scope::{Polarity, Pred, Scopes};
use crate::versioning::episode::EpisodeKey;

/// The data `ties` cover: every datum when one of them names no episode,
/// none when there are none.
pub(crate) fn covered<'t>(judged: &Judged<'_>, ties: impl IntoIterator<Item = &'t Tie>) -> Pred {
    let mut keys: Vec<EpisodeKey> = Vec::new();
    let mut any = false;
    for tie in ties {
        any = true;
        match tie.episode() {
            Some(key) => keys.push(key),
            None => return Pred::True,
        }
    }
    if !any {
        return Pred::False;
    }
    keys.sort_unstable();
    keys.dedup();
    beside(judged, Pred::InEpisodes(Arc::from(keys)))
}

/// The data the caller's live relationships to the subject cover, of those
/// that meet `need` when one is named, left to a query's statement.
pub(crate) fn left(judged: &Judged<'_>, need: Option<CareNeed>) -> Pred {
    beside(judged, Pred::Covered(need.map(Arc::new)))
}

/// `episodes`, the data a relationship covers by its episode, with the data
/// every episode covers.
fn beside(judged: &Judged<'_>, episodes: Pred) -> Pred {
    Pred::any(vec![
        episodes,
        shared(judged),
        if judged.caller.kind == CallerKind::AccessService {
            judged.scopes.priority()
        } else {
            Pred::False
        },
    ])
}

/// The data `ACCESS_POLICY.episode_shared` names: none when it is void.
pub(crate) fn shared(judged: &Judged<'_>) -> Pred {
    shared_of(judged.policy.episode_shared, judged.scopes)
}

/// The data the scopes `shared` of `ACCESS_POLICY.episode_shared` name, as
/// `scopes` read them: none when it is void.
pub(crate) fn shared_of(shared: &[AccessDataScope], scopes: &Scopes) -> Pred {
    // NOTE: draft BMM ACCESS_POLICY.episode_shared, void means no data are shared, where an
    // empty data list elsewhere names the whole EHR.
    if shared.is_empty() {
        Pred::False
    } else {
        scopes.data(shared, Polarity::Permit, None)
    }
}
