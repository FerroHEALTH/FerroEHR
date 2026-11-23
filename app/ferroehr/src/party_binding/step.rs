// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The statements that write the binding side table and the grant rows, each
//! a [`Step`] run alone or folded with others into one statement.
//!
//! A step is one data-modifying statement with its own parameters from `$1`.
//! Run alone it is sent as it is written. [`batch`] folds several into the
//! data-modifying CTEs of one statement, each placeholder moved past the
//! steps before it, so a party commit settles its rows in ordered statements:
//! every delete and retirement, then every insert (#6257). The CTEs of one
//! statement see one snapshot and run in no order the statement fixes
//! (<https://www.postgresql.org/docs/18/queries-with.html#QUERIES-WITH-MODIFYING>),
//! so a batch only folds steps that write disjoint rows. No openEHR spec
//! governs the side table or its SQL: our own design.

use std::borrow::Cow;
use std::fmt::Write as _;

use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::storage::error::StorageError;

/// A statement whose parameters a [`Step`] binds.
pub(crate) type Query<'q> = sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>;

/// What the rows a [`Step`] returns say; a tagged step returns `p` (a
/// container) and `t` (a row's text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tag {
    /// A caller fact that went: the party it is about and its text.
    FactsGone,
    /// A caller fact that came: the party it is about and its text.
    FactsCame,
    /// A record subject that went, with its text.
    SubjectsGone,
    /// A record subject that came, with its text.
    SubjectsCame,
    /// A client, provider or capability row that went, came or changed.
    Grants,
    /// A `GROUP` one of whose membership rows was retired.
    Groups,
    /// The representative a retired representation row named.
    Representatives,
}

impl Tag {
    /// Every tag, at its code.
    const ALL: [Self; 7] = [
        Self::FactsGone,
        Self::FactsCame,
        Self::SubjectsGone,
        Self::SubjectsCame,
        Self::Grants,
        Self::Groups,
        Self::Representatives,
    ];

    /// The code the folded statement selects for it.
    fn code(self) -> i16 {
        match self {
            Self::FactsGone => 0,
            Self::FactsCame => 1,
            Self::SubjectsGone => 2,
            Self::SubjectsCame => 3,
            Self::Grants => 4,
            Self::Groups => 5,
            Self::Representatives => 6,
        }
    }

    /// The tag of `code`.
    fn of(code: i16) -> Option<Self> {
        Self::ALL.into_iter().find(|tag| tag.code() == code)
    }
}

/// One row a tagged step returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Returned {
    /// What the row says.
    pub(crate) tag: Tag,
    /// The container it names.
    pub(crate) party: Option<Uuid>,
    /// Its text, for the rows a commit compares before and after.
    pub(crate) text: Option<String>,
}

/// The parameters a step binds, in placeholder order.
type Binder<'q> = Box<dyn FnOnce(Query<'q>) -> Query<'q> + Send + 'q>;

/// One data-modifying statement of the side-table writes, named for the CTE
/// it becomes in a [`batch`].
pub(crate) struct Step<'q> {
    /// Its CTE name, unique within a batch.
    name: &'static str,
    /// Its text, with its placeholders from `$1`.
    sql: Cow<'static, str>,
    /// What its rows say; `None` for a step whose rows nobody reads.
    tag: Option<Tag>,
    /// Whether it may write a row; `false` for an insert of no rows.
    writes: bool,
    /// Binds its parameters.
    bind: Binder<'q>,
}

impl std::fmt::Debug for Step<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Step")
            .field("name", &self.name)
            .field("tag", &self.tag)
            .field("writes", &self.writes)
            .finish_non_exhaustive()
    }
}

impl<'q> Step<'q> {
    /// The step `name` of the statement `sql`, whose placeholders `bind`
    /// binds from `$1` on, every one up to the highest it names; with a
    /// `tag`, it returns `p` and `t`.
    pub(crate) fn new(
        name: &'static str,
        sql: impl Into<Cow<'static, str>>,
        tag: Option<Tag>,
        bind: impl FnOnce(Query<'q>) -> Query<'q> + Send + 'q,
    ) -> Self {
        Self {
            name,
            sql: sql.into(),
            tag,
            writes: true,
            bind: Box::new(bind),
        }
    }

    /// The step, marked as one that writes no row when `writes` is `false`:
    /// an insert of no rows, which a [`batch`] of such steps alone never
    /// sends.
    pub(crate) fn writing(mut self, writes: bool) -> Self {
        self.writes = writes;
        self
    }

    /// Runs the step alone on `tx`, returning the rows of a tagged step.
    ///
    /// # Errors
    /// [`StorageError::Database`] on a driver failure.
    pub(crate) async fn run(self, tx: &mut PgConnection) -> Result<Vec<Returned>, StorageError> {
        let Self { sql, tag, bind, .. } = self;
        let rows = bind(sqlx::query(sqlx::AssertSqlSafe(sql)))
            .fetch_all(&mut *tx)
            .await?;
        let Some(tag) = tag else {
            return Ok(Vec::new());
        };
        rows.iter()
            .map(|row| {
                Ok(Returned {
                    tag,
                    party: row.try_get("p")?,
                    text: row.try_get("t")?,
                })
            })
            .collect()
    }
}

/// Runs `steps` as the CTEs of one statement on `tx`, returning the rows of
/// the tagged ones; nothing is sent when no step may write a row, and a lone
/// step is sent as it is written.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub(crate) async fn batch(
    tx: &mut PgConnection,
    mut steps: Vec<Step<'_>>,
) -> Result<Vec<Returned>, StorageError> {
    if !steps.iter().any(|step| step.writes) {
        return Ok(Vec::new());
    }
    if steps.len() == 1
        && let Some(step) = steps.pop()
    {
        return step.run(tx).await;
    }
    let mut sql = String::with_capacity(
        steps
            .iter()
            .map(|step| step.sql.len().saturating_add(64))
            .sum(),
    );
    let mut taken = 0_usize;
    for step in &steps {
        sql.push_str(if sql.is_empty() { "WITH " } else { ", " });
        sql.push_str(step.name);
        sql.push_str(" AS (");
        let highest = shift_into(&mut sql, &step.sql, taken);
        sql.push(')');
        taken = taken.saturating_add(highest);
    }
    sql.push_str(" SELECT NULL::smallint AS tag, NULL::uuid AS p, NULL::text AS t WHERE false");
    for step in &steps {
        if let Some(tag) = step.tag {
            let _ = write!(
                sql,
                " UNION ALL SELECT {}::smallint, p, t FROM {}",
                tag.code(),
                step.name
            );
        }
    }
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for step in steps {
        query = (step.bind)(query);
    }
    let rows = query.fetch_all(&mut *tx).await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let code: i16 = row.try_get("tag")?;
        let Some(tag) = Tag::of(code) else {
            return Err(StorageError::InvalidRows(format!(
                "a folded side-table statement returned the tag {code}, which no step selects"
            )));
        };
        out.push(Returned {
            tag,
            party: row.try_get("p")?,
            text: row.try_get("t")?,
        });
    }
    Ok(out)
}

/// The containers the rows of `tag` among `rows` name.
pub(crate) fn parties(rows: &[Returned], tag: Tag) -> Vec<Uuid> {
    rows.iter()
        .filter(|row| row.tag == tag)
        .filter_map(|row| row.party)
        .collect()
}

/// Whether the texts of the rows of `gone` differ from those of `came`
/// among `rows`, in any order: the rows of a head changed.
pub(crate) fn differ(rows: &[Returned], gone: Tag, came: Tag) -> bool {
    let texts = |tag: Tag| {
        let mut texts: Vec<Option<&str>> = rows
            .iter()
            .filter(|row| row.tag == tag)
            .map(|row| row.text.as_deref())
            .collect();
        texts.sort_unstable();
        texts
    };
    texts(gone) != texts(came)
}

/// Pushes `sql` onto `out` with every placeholder `$n` moved to `$n + by`,
/// returning the highest placeholder it names.
fn shift_into(out: &mut String, sql: &str, by: usize) -> usize {
    let mut highest = 0_usize;
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c != '$' {
            continue;
        }
        let mut number: Option<usize> = None;
        while let Some(digit) = chars.peek().and_then(|d| d.to_digit(10)) {
            let digit = usize::try_from(digit).unwrap_or(0);
            number = Some(number.unwrap_or(0).saturating_mul(10).saturating_add(digit));
            chars.next();
        }
        if let Some(number) = number {
            highest = highest.max(number);
            let _ = write!(out, "{}", number.saturating_add(by));
        }
    }
    highest
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `sql` shifted by `by`, with the highest placeholder it names.
    fn shifted(sql: &str, by: usize) -> (String, usize) {
        let mut out = String::new();
        let highest = shift_into(&mut out, sql, by);
        (out, highest)
    }

    #[test]
    fn a_shift_moves_every_placeholder_and_names_the_highest() {
        let (sql, highest) = shifted("SELECT $1, $12::text[], '$x' WHERE a = $3", 4);
        assert_eq!(sql, "SELECT $5, $16::text[], '$x' WHERE a = $7");
        assert_eq!(highest, 12);
    }

    #[test]
    fn a_shift_by_none_leaves_the_text() {
        let text = "UPDATE t SET a = $1 WHERE b ~ '^x{2}$'";
        assert_eq!(shifted(text, 0), (text.to_owned(), 1));
    }

    #[test]
    fn every_tag_reads_back_from_its_code() {
        for tag in Tag::ALL {
            assert_eq!(Tag::of(tag.code()), Some(tag));
        }
    }

    #[test]
    fn rows_differ_by_their_texts_in_any_order() {
        let row = |tag, text: &str| Returned {
            tag,
            party: None,
            text: Some(text.to_owned()),
        };
        let same = [
            row(Tag::FactsGone, "a"),
            row(Tag::FactsGone, "b"),
            row(Tag::FactsCame, "b"),
            row(Tag::FactsCame, "a"),
        ];
        assert!(!differ(&same, Tag::FactsGone, Tag::FactsCame));
        let moved = [row(Tag::FactsGone, "a"), row(Tag::FactsCame, "c")];
        assert!(differ(&moved, Tag::FactsGone, Tag::FactsCame));
    }
}
