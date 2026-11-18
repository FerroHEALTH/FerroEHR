// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading the rows of an AQL statement back into rendered cells.
//!
//! The column positions and SQL types are resolved once per page from its
//! first row; every row then renders each cell once
//! ([`crate::aql::cell::Cell`]). A data leaf's node row is rebuilt into its
//! fragment at most once per row however many cells read it. No openEHR spec
//! governs the read-back: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 5): AQL result cells are arbitrary \
              projections of a stored fragment by specification (QUERY 1.1)"
)]

use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Column as _, Row as _, TypeInfo as _, ValueRef as _};

use openehr_base::v1_3::base_types::identification::object_version_id::ObjectVersionId;

use super::cell::{self, Cell};
use super::error::{AqlError, ExecError};
use super::fragment::FragmentPath;
use super::fragment::write_jsonb;
use super::sql::{CellKind, ColumnRender, ColumnSpec, FragmentCell, Layout};

/// The resolved read plan of one page: per column its SQL column positions
/// and, for a typed column, its SQL type; per node group its first position.
pub(super) struct Plan {
    columns: Vec<(Vec<usize>, Option<Typed>)>,
    groups: Vec<usize>,
    /// Per column, the position of its debug-build reference cell.
    references: Vec<Option<usize>>,
}

/// The SQL type of a [`ColumnRender::Typed`] column.
#[derive(Clone, Copy)]
enum Typed {
    Text,
    Int2,
    Int4,
    Int8,
    Numeric,
    Float4,
    Float8,
    Bool,
    Uuid,
    Jsonb,
}

impl Plan {
    /// The read plan of `layout` over the page whose first row is `row`.
    ///
    /// # Errors
    /// [`ExecError`] when a declared column is missing or a typed column has
    /// no rendering.
    pub(super) fn of(row: &PgRow, layout: &Layout) -> Result<Self, AqlError> {
        let position = |name: &str| -> Result<usize, AqlError> {
            Ok(row.try_column(name).map_err(ExecError::from)?.ordinal())
        };
        let groups = layout
            .node_groups
            .iter()
            .map(|group| position(&group.first))
            .collect::<Result<Vec<_>, _>>()?;
        let columns = layout
            .columns
            .iter()
            .map(|spec| {
                let at = spec
                    .sql_cols
                    .iter()
                    .map(|name| position(name))
                    .collect::<Result<Vec<_>, _>>()?;
                let typed = match spec.kind {
                    CellKind::Column(ColumnRender::Typed) => {
                        let first = first_col(spec, &at)?;
                        Some(typed(spec, row.column(first).type_info().name())?)
                    }
                    _ => None,
                };
                Ok((at, typed))
            })
            .collect::<Result<Vec<_>, AqlError>>()?;
        let references = layout
            .columns
            .iter()
            .map(|spec| spec.reference.as_deref().map(position).transpose())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            columns,
            groups,
            references,
        })
    }

    /// The cells of `row`; a whole-object cell is `null` until the executor
    /// fills it.
    ///
    /// # Errors
    /// [`ExecError`] when a value cannot be read back.
    pub(super) fn cells(&self, row: &PgRow, layout: &Layout) -> Result<Vec<Cell>, AqlError> {
        let mut fragments: Vec<GroupRow> = vec![GroupRow::default(); self.groups.len()];
        let mut cells = Vec::with_capacity(layout.columns.len());
        for (spec, (at, typed)) in layout.columns.iter().zip(&self.columns) {
            let cell = match &spec.kind {
                CellKind::Scalar => {
                    let v: Option<Value> =
                        row.try_get(first_col(spec, at)?).map_err(ExecError::from)?;
                    v.as_ref().map_or_else(Cell::null, Cell::of_value)
                }
                CellKind::WholeObject => Cell::null(),
                CellKind::Literal(lit) => cell::literal(lit),
                CellKind::Column(ColumnRender::Typed) => {
                    let ty = typed.ok_or_else(|| missing(spec, 0))?;
                    typed_cell(row, first_col(spec, at)?, ty)?
                }
                CellKind::Column(ColumnRender::Timestamp) => timestamp_cell(row, spec, at)?,
                CellKind::Column(ColumnRender::VersionUid) => {
                    let uid: Option<String> =
                        row.try_get(first_col(spec, at)?).map_err(ExecError::from)?;
                    match uid {
                        Some(uid) => version_uid_cell(uid)?,
                        None => Cell::null(),
                    }
                }
                CellKind::Fragment(cell) => {
                    self.fragment_cell(row, spec, at, cell, &mut fragments)?
                }
            };
            if let Some(Some(at)) = self.references.get(cells.len()) {
                let reference: Option<Value> = row.try_get(*at).map_err(ExecError::from)?;
                debug_assert_eq!(
                    cell.json(),
                    reference
                        .as_ref()
                        .map_or_else(Cell::null, Cell::of_value)
                        .json(),
                    "column `{}`: the Rust-rendered cell is the jsonb cell PostgreSQL builds",
                    spec.name
                );
            }
            cells.push(cell);
        }
        Ok(cells)
    }

    /// The cell a data leaf reads off its node group's rebuilt fragment.
    fn fragment_cell(
        &self,
        row: &PgRow,
        spec: &ColumnSpec,
        at: &[usize],
        fc: &FragmentCell,
        fragments: &mut [GroupRow],
    ) -> Result<Cell, AqlError> {
        if fc.guarded {
            let held: Option<bool> = row.try_get(first_col(spec, at)?).map_err(ExecError::from)?;
            if held != Some(true) {
                return Ok(Cell::null());
            }
        }
        let start = *self.groups.get(fc.group).ok_or_else(|| missing(spec, 0))?;
        let rm_type: Option<&str> = row.try_get(start + 1).map_err(ExecError::from)?;
        let Some(rm_type) = rm_type else {
            // The row binds no node (an outer-joined absent step).
            return Ok(Cell::null());
        };
        let slot = fragments
            .get_mut(fc.group)
            .ok_or_else(|| missing(spec, 0))?;
        let Some(path) = &fc.path else {
            return Ok(Cell::of_jsonb(slot.full(row, start)?));
        };
        let reached = match path.first_member() {
            Some(first) => slot.member(row, start, rm_type, first, path)?,
            None => Member::Unread,
        };
        let items = match reached {
            Member::Absent => Vec::new(),
            Member::Written(cell) => return Ok(cell),
            Member::Borrowed(value) if !fc.list => {
                return Ok(path
                    .first_below(value)
                    .map_or_else(Cell::null, Cell::of_jsonb));
            }
            Member::Owned(value) if !fc.list => {
                return Ok(path
                    .first_below(&value)
                    .map_or_else(Cell::null, Cell::of_jsonb));
            }
            Member::Borrowed(value) => return Ok(list_cell(&path.all_below(value))),
            Member::Owned(value) => return Ok(list_cell(&path.all_below(&value))),
            Member::Unread => {
                let fragment = slot.full(row, start)?;
                if !fc.list {
                    return Ok(path.first(fragment).map_or_else(Cell::null, Cell::of_jsonb));
                }
                path.all(fragment)
            }
        };
        Ok(list_cell(&items))
    }
}

/// The array cell of every match, `null` for none.
fn list_cell(items: &[&Value]) -> Cell {
    if items.is_empty() {
        return Cell::null();
    }
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_jsonb(item, &mut out);
    }
    out.push(']');
    Cell::of_text(out)
}

/// The members a node row's columns or markers put back into its fragment
/// (`crate::storage::slim::rebuild`), other than `_type`, `value`, `origin`
/// and `time`, which [`GroupRow::member`] reads directly.
const RESTORED: [&str; 7] = [
    "name",
    "archetype_node_id",
    "archetype_details",
    "language",
    "encoding",
    "subject",
    "uid",
];

/// One member of a node row's fragment, read without rebuilding the rest.
enum Member<'s> {
    /// The fragment has no such member.
    Absent,
    /// The member's value as stored in `data`.
    Borrowed(&'s Value),
    /// The member's value as the columns put it back.
    Owned(Value),
    /// The cell itself, written from the columns.
    Written(Cell),
    /// Only the whole rebuilt fragment answers.
    Unread,
}

/// A node group's stored values in one row, each read on first use.
#[derive(Clone, Default)]
pub(super) struct GroupRow {
    /// The stored `data`, once read.
    data: Option<Value>,
    data_read: bool,
    /// The whole rebuilt fragment, once rebuilt.
    full: Option<Value>,
}

impl GroupRow {
    /// The member `name` of the row's fragment as the rebuild puts it back,
    /// read off the columns and `data` alone where they hold it whole, the
    /// reads the SQL projection specialised (`aql::sql::expr::fragment_source`):
    /// `_type` from `rm_type`, `value` from the typed value columns, a
    /// compact `origin` or `time` from its marker, and a member no column
    /// restores from `data`.
    ///
    /// # Errors
    /// [`ExecError`] when a stored column cannot be read back.
    fn member(
        &mut self,
        row: &PgRow,
        start: usize,
        rm_type: &str,
        name: &str,
        path: &FragmentPath,
    ) -> Result<Member<'_>, AqlError> {
        match name {
            "_type" => return Ok(Member::Owned(Value::from(rm_type))),
            "value" => {
                let value_type: Option<&str> = row.try_get(start + 6).map_err(ExecError::from)?;
                if let Some(value_type) = value_type {
                    return typed_member(row, start, value_type, path);
                }
            }
            "origin" | "time" => {
                let column = if name == "origin" { 14 } else { 15 };
                let marked: Option<bool> = row.try_get(start + column).map_err(ExecError::from)?;
                if marked == Some(true) {
                    let stored = self.data(row, start)?.and_then(|data| data.get(name));
                    // NOTE: aql::sql::compact, a marked member's `value` is the stored string.
                    if path.is_exactly(&[name, "value"]) {
                        return Ok(Member::Written(
                            stored.map_or_else(Cell::null, Cell::of_jsonb),
                        ));
                    }
                    return Ok(Member::Owned(crate::storage::slim::object([
                        ("_type", Value::from("DV_DATE_TIME")),
                        ("value", stored.cloned().unwrap_or(Value::Null)),
                    ])));
                }
            }
            _ if RESTORED.contains(&name) => return Ok(Member::Unread),
            _ => {}
        }
        Ok(
            match self.data(row, start)?.and_then(|data| data.get(name)) {
                Some(value) => Member::Borrowed(value),
                None => Member::Absent,
            },
        )
    }

    /// The stored `data`, read once.
    ///
    /// # Errors
    /// [`ExecError`] when the column cannot be read back.
    fn data(&mut self, row: &PgRow, start: usize) -> Result<Option<&Value>, AqlError> {
        if !self.data_read {
            self.data = row.try_get(start).map_err(ExecError::from)?;
            self.data_read = true;
        }
        Ok(self.data.as_ref())
    }

    /// The whole rebuilt fragment, rebuilt once.
    ///
    /// # Errors
    /// [`ExecError`] when a stored column cannot be read back.
    fn full(&mut self, row: &PgRow, start: usize) -> Result<&Value, AqlError> {
        if self.full.is_none() {
            self.full =
                Some(crate::storage::node_repo::row_fragment(row, start).map_err(ExecError::from)?);
        }
        self.full.as_ref().ok_or_else(|| {
            ExecError::from(crate::storage::error::StorageError::InvalidRows(
                "a node row's fragment did not rebuild".into(),
            ))
            .into()
        })
    }
}

/// The `value` member of an `ELEMENT` row from its typed value columns: the
/// cell itself, written straight from the columns, when the path is exactly
/// `value`, and the rebuilt value (`crate::storage::slim::rebuild_value`)
/// for a path below it.
///
/// # Errors
/// [`ExecError`] when a column cannot be read back or a typed number is no
/// JSON number.
fn typed_member<'s>(
    row: &PgRow,
    start: usize,
    value_type: &str,
    path: &FragmentPath,
) -> Result<Member<'s>, AqlError> {
    let get = |i: usize| -> Result<Option<&str>, AqlError> {
        Ok(row.try_get(start + i).map_err(ExecError::from)?)
    };
    let (num, text, code, terminology) = (get(7)?, get(8)?, get(9)?, get(10)?);
    let no_number = || {
        AqlError::from(ExecError::from(
            crate::storage::error::StorageError::InvalidRows(
                "a node row holds a typed number that is no JSON number".into(),
            ),
        ))
    };
    if path.is_exactly(&["value"]) {
        let mut out = String::with_capacity(96);
        cell::write_typed_value(&mut out, value_type, num, text, code, terminology)
            .ok_or_else(no_number)?;
        return Ok(Member::Written(Cell::of_text(out)));
    }
    let typed = crate::storage::slim::TypedValue {
        value_type: value_type.to_owned(),
        num: num.map(str::to_owned),
        text: text.map(str::to_owned),
        code: code.map(str::to_owned),
        terminology: terminology.map(str::to_owned),
    };
    crate::storage::slim::rebuild_value(&typed)
        .map(Member::Owned)
        .ok_or_else(no_number)
}

/// The cell of a version's `OBJECT_VERSION_ID` from its uid text.
///
/// # Errors
/// [`ExecError::Assembly`] when the stored identity columns compose no
/// `OBJECT_VERSION_ID`.
fn version_uid_cell(uid: String) -> Result<Cell, AqlError> {
    let id = ObjectVersionId::new(uid).map_err(|e| {
        ExecError::from(crate::storage::error::StorageError::InvalidRows(format!(
            "a version row composes no OBJECT_VERSION_ID: {e}"
        )))
    })?;
    let value =
        serde_json::to_value(&id).map_err(|e| ExecError::from(sqlx::Error::Decode(e.into())))?;
    Ok(Cell::of_jsonb(&value))
}

/// The rendering of the SQL type `name` of `spec`'s column.
///
/// # Errors
/// [`ExecError::UnrenderedType`] for a type with none.
fn typed(spec: &ColumnSpec, name: &str) -> Result<Typed, AqlError> {
    Ok(match name {
        "TEXT" | "VARCHAR" | "BPCHAR" | "NAME" => Typed::Text,
        "INT2" => Typed::Int2,
        "INT4" => Typed::Int4,
        "INT8" => Typed::Int8,
        "NUMERIC" => Typed::Numeric,
        "FLOAT4" => Typed::Float4,
        "FLOAT8" => Typed::Float8,
        "BOOL" => Typed::Bool,
        "UUID" => Typed::Uuid,
        "JSONB" | "JSON" => Typed::Jsonb,
        other => {
            return Err(ExecError::UnrenderedType {
                column: spec.name.clone(),
                sql_type: other.to_owned(),
            }
            .into());
        }
    })
}

/// The cell of a typed column at `at`.
///
/// # Errors
/// [`ExecError`] when the value cannot be decoded as its type.
fn typed_cell(row: &PgRow, at: usize, ty: Typed) -> Result<Cell, AqlError> {
    fn get<'r, T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>>(
        row: &'r PgRow,
        at: usize,
    ) -> Result<Option<T>, AqlError> {
        Ok(row.try_get(at).map_err(ExecError::from)?)
    }
    Ok(match ty {
        Typed::Text => get::<&str>(row, at)?.map(Cell::of_str),
        Typed::Int2 => get::<i16>(row, at)?.map(|n| Cell::of_value(&Value::from(n))),
        Typed::Int4 => get::<i32>(row, at)?.map(|n| Cell::of_value(&Value::from(n))),
        Typed::Int8 => get::<i64>(row, at)?.map(|n| Cell::of_value(&Value::from(n))),
        Typed::Float4 => get::<f32>(row, at)?.map(|n| cell::float(f64::from(n))),
        Typed::Float8 => get::<f64>(row, at)?.map(cell::float),
        Typed::Bool => get::<bool>(row, at)?.map(|b| Cell::of_value(&Value::from(b))),
        Typed::Uuid => get::<uuid::Uuid>(row, at)?.map(|u| Cell::of_str(&u.to_string())),
        Typed::Jsonb => get::<Value>(row, at)?.map(|v| Cell::of_jsonb(&v)),
        Typed::Numeric => {
            let raw = row.try_get_raw(at).map_err(ExecError::from)?;
            if raw.is_null() {
                None
            } else {
                let bytes = raw
                    .as_bytes()
                    .map_err(|e| ExecError::from(sqlx::Error::Decode(e)))?;
                let text = cell::numeric_text(bytes).ok_or_else(|| {
                    ExecError::from(sqlx::Error::Decode("a malformed numeric value".into()))
                })?;
                Some(cell::numeric(&text))
            }
        }
    }
    .unwrap_or_else(Cell::null))
}

/// The cell of a `timestamptz` column beside its session offset.
///
/// # Errors
/// [`ExecError`] when either column cannot be read back.
fn timestamp_cell(row: &PgRow, spec: &ColumnSpec, at: &[usize]) -> Result<Cell, AqlError> {
    let value = first_col(spec, at)?;
    let offset = *at.get(1).ok_or_else(|| missing(spec, 1))?;
    let at_value: Option<jiff_sqlx::Timestamp> = row.try_get(value).map_err(ExecError::from)?;
    let seconds: Option<i32> = row.try_get(offset).map_err(ExecError::from)?;
    Ok(match (at_value, seconds) {
        (Some(ts), Some(seconds)) => cell::timestamp(ts.to_jiff(), seconds),
        _ => Cell::null(),
    })
}

/// The first SQL column position of `spec`.
///
/// # Errors
/// [`ExecError::MissingColumnAlias`] when it declares none.
fn first_col(spec: &ColumnSpec, at: &[usize]) -> Result<usize, AqlError> {
    at.first().copied().ok_or_else(|| missing(spec, 0))
}

/// The refusal of a column spec without the alias `index`.
fn missing(spec: &ColumnSpec, index: usize) -> AqlError {
    ExecError::MissingColumnAlias {
        column: spec.name.clone(),
        index,
    }
    .into()
}
