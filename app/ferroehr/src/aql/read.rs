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

/// A node group's fragment in one row, rebuilt on first use.
#[derive(Clone)]
enum Rebuilt {
    /// Not read yet.
    Pending,
    /// The row binds no node.
    Absent,
    /// The rebuilt fragment.
    Fragment(Value),
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
        let mut fragments: Vec<Rebuilt> = vec![Rebuilt::Pending; self.groups.len()];
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
        fragments: &mut [Rebuilt],
    ) -> Result<Cell, AqlError> {
        if fc.guarded {
            let held: Option<bool> = row.try_get(first_col(spec, at)?).map_err(ExecError::from)?;
            if held != Some(true) {
                return Ok(Cell::null());
            }
        }
        let slot = fragments
            .get_mut(fc.group)
            .ok_or_else(|| missing(spec, 0))?;
        if matches!(slot, Rebuilt::Pending) {
            let start = *self.groups.get(fc.group).ok_or_else(|| missing(spec, 0))?;
            *slot = fragment(row, start)?.map_or(Rebuilt::Absent, Rebuilt::Fragment);
        }
        let Rebuilt::Fragment(fragment) = &*slot else {
            return Ok(Cell::null());
        };
        let Some(path) = &fc.path else {
            return Ok(Cell::of_jsonb(fragment));
        };
        if !fc.list {
            return Ok(path.first(fragment).map_or_else(Cell::null, Cell::of_jsonb));
        }
        let items = path.all(fragment);
        if items.is_empty() {
            return Ok(Cell::null());
        }
        let mut out = String::from("[");
        for (i, item) in items.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_jsonb(item, &mut out);
        }
        out.push(']');
        Ok(Cell::of_text(out))
    }
}

/// The rebuilt fragment of the node group from column `start`, `None` when
/// the row binds no node (an outer-joined absent step).
///
/// # Errors
/// [`ExecError`] when a stored column cannot be read back.
fn fragment(row: &PgRow, start: usize) -> Result<Option<Value>, AqlError> {
    let rm_type: Option<String> = row.try_get(start + 1).map_err(ExecError::from)?;
    if rm_type.is_none() {
        return Ok(None);
    }
    Ok(Some(
        crate::storage::node_repo::row_fragment(row, start).map_err(ExecError::from)?,
    ))
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
