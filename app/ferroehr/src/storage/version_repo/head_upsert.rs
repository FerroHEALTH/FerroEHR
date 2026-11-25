// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The head-row upsert every folded commit statement carries, and what it
//! does with the head row's episodes (#6413). No openEHR spec governs storage:
//! our own design.

use std::borrow::Cow;

use super::commit::FoldedVersion;

/// What a version's head upsert does with the head row's `episodes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeadEpisodes {
    /// Leaves them as they are: a first version placed in none, or a version
    /// of a kind no episode places.
    Untouched,
    /// Clears them on a trunk commit: a later COMPOSITION version placed in
    /// none, which moves the object out of its episodes.
    Cleared,
    /// Writes the parameter of this number on a trunk commit.
    Placed(usize),
}

/// What a version does with its object's episodes, the three statement
/// variants a shape is built in ([`HeadEpisodes`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placement {
    /// [`HeadEpisodes::Untouched`].
    Untouched = 0,
    /// [`HeadEpisodes::Cleared`].
    Cleared = 1,
    /// [`HeadEpisodes::Placed`].
    Placed = 2,
}

impl Placement {
    /// Every variant, in the order of its discriminant.
    pub(crate) const ALL: [Self; 3] = [Self::Untouched, Self::Cleared, Self::Placed];

    /// What `v` does with its object's episodes.
    pub(crate) fn of(v: &FoldedVersion<'_>) -> Self {
        if !v.episodes.is_empty() {
            Self::Placed
        } else if v.kind == "COMPOSITION" && v.preceding_version_uid.is_some() {
            Self::Cleared
        } else {
            Self::Untouched
        }
    }

    /// The statement text among the three of `texts` this variant runs.
    pub(crate) fn pick(self, texts: &[String; 3]) -> &str {
        match self {
            Self::Untouched => &texts[0],
            Self::Cleared => &texts[1],
            Self::Placed => &texts[2],
        }
    }
}

/// The head upsert both folded commit statements carry.
///
/// A TRUNK commit advances every column; a BRANCH commit advances only the
/// any-lineage head ordinal and the commit instant, since a branch does not
/// supersede the trunk (RM common master06 §The 'Virtual Version Tree'). A
/// create inserts the row ([`Self::insert`]); every later version updates it
/// ([`Self::advance`]). The update applies only when the committing ordinal
/// is greater than the recorded one, so a replayed statement never walks the
/// head backwards.
///
/// `{branch}`, `{ordinal}`, `{lifecycle}`, `{template}`, `{archetype}` and
/// the episodes of [`HeadEpisodes::Placed`] are the caller's parameter
/// placeholders; `FROM {version}` orders each CTE after the version insert.
pub(crate) struct HeadUpsert<'a> {
    /// The versioned object's id.
    pub(crate) vo: &'a str,
    /// The versioned object's RM type.
    pub(crate) kind: &'a str,
    /// The owning EHR, `NULL` for a party.
    pub(crate) ehr: &'a str,
    /// The new version's `sys_version`.
    pub(crate) ordinal: &'a str,
    /// The new version's `branch_number`; `0` is a trunk commit.
    pub(crate) branch: &'a str,
    /// The new version's `lifecycle_state`.
    pub(crate) lifecycle: &'a str,
    /// The template the new version was validated against.
    pub(crate) template: &'a str,
    /// The root archetype of the new version's content.
    pub(crate) archetype: &'a str,
    /// What the upsert does with the head row's episodes.
    pub(crate) episodes: HeadEpisodes,
    /// The commit instant.
    pub(crate) committed_at: &'a str,
    /// The CTE that inserted the version row, which orders this one after it.
    pub(crate) version: &'a str,
}

impl HeadUpsert<'_> {
    /// The monotonic upsert of a later version's head row, which writes a row
    /// for an ordinal above 1 alone.
    pub(crate) fn advance(&self) -> String {
        let Self {
            branch, episodes, ..
        } = *self;
        let (set_open, set_branch, set_close) = match episodes {
            HeadEpisodes::Untouched => ("", "", ""),
            HeadEpisodes::Cleared | HeadEpisodes::Placed(_) => (
                ", episodes = CASE WHEN ",
                branch,
                " = 0 THEN EXCLUDED.episodes ELSE vo_head.episodes END",
            ),
        };
        format!(
            "{} \
         ON CONFLICT (vo_id) DO UPDATE SET \
             head_sys_version = EXCLUDED.head_sys_version, \
             committed_at = EXCLUDED.committed_at, \
             trunk_head_sys_version = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.head_sys_version ELSE vo_head.trunk_head_sys_version END, \
             trunk_head_content_key = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.trunk_head_content_key ELSE vo_head.trunk_head_content_key END, \
             lifecycle_state = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.lifecycle_state ELSE vo_head.lifecycle_state END, \
             template_id = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.template_id ELSE vo_head.template_id END, \
             root_archetype = CASE WHEN {branch} = 0 \
                 THEN EXCLUDED.root_archetype ELSE vo_head.root_archetype END{set_open}{set_branch}{set_close} \
         WHERE EXCLUDED.head_sys_version > vo_head.head_sys_version \
           AND ({branch} <> 0 \
                OR vo_head.trunk_head_sys_version IS NULL \
                OR EXCLUDED.head_sys_version > vo_head.trunk_head_sys_version)",
            self.insert("> 1")
        )
    }

    /// The head row's insert, which writes a row for an ordinal `ordinals`
    /// admits (`= 1`, a create).
    pub(crate) fn insert(&self, ordinals: &str) -> String {
        let Self {
            vo,
            kind,
            ehr,
            ordinal,
            branch,
            lifecycle,
            template,
            archetype,
            episodes,
            committed_at,
            version,
        } = *self;
        let (episodes_column, episodes_value) = match episodes {
            HeadEpisodes::Placed(at) => (", episodes", Cow::Owned(format!(", ${at}::bytea[]"))),
            HeadEpisodes::Untouched | HeadEpisodes::Cleared => ("", Cow::Borrowed("")),
        };
        format!(
            "INSERT INTO vo_head (vo_id, kind, ehr_id, head_sys_version, \
             trunk_head_sys_version, trunk_head_content_key, lifecycle_state, template_id, \
             root_archetype{episodes_column}, committed_at) \
         SELECT {vo}, {kind}, {ehr}, {ordinal}, \
                CASE WHEN {branch} = 0 THEN {ordinal} ELSE NULL END, \
                CASE WHEN {branch} = 0 THEN {version}.content_key ELSE NULL END, \
                {lifecycle}, {template}, {archetype}{episodes_value}, {committed_at}::timestamptz \
         FROM {version} WHERE {ordinal}::int {ordinals}"
        )
    }
}
