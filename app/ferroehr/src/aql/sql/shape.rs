// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Statement shapes: the SQL text of a plan rendered once per shape, and its
//! values bound from numbered slots.
//!
//! A cached plan ([`crate::service::query`]) spares a repeated query its parse
//! and lowering, but [`super::build`] still builds the whole `sea-query` tree
//! and renders it on every request. The text depends on more than the plan:
//! the parameters, the scope, the paging and the access condition shape it.
//! What varies between the requests of one caller, or of callers that share a
//! verdict, are values: the EHR a single-EHR query names, the request's
//! moment and day, the caller's parties and care teams, the EHRs it is cared
//! for or confined to, the pinned EHR's revision, the `ehr_ids` scope and the
//! paging bounds. A [`Live`] source names each of them.
//!
//! [`Shapes`] holds, per plan, the shapes the plan has been executed in. A
//! request whose [`Key`] matches a shape binds the shape's slots from its own
//! values and renders nothing; a request that matches none is built as before.
//! The second request of a key derives its shape from two builds over
//! [`Sentinels`]: every [`Live`] source carries a fresh random value in each,
//! the two texts must be equal, and every value either agrees in both (a fixed
//! value of the shape) or is the image of one [`Live`] source in each (a slot).
//! A value that depends on a source any other way, or a text that does, leaves
//! the key unshaped, and the shape must reproduce the request's own build
//! exactly before it is kept. The dataflow is thus observed rather than
//! declared, so a value the emitter derives in a way this module does not know
//! costs the cache, never a wrong statement.
//!
//! No openEHR spec governs the SQL build: our own design (#5783).

mod key;
mod live;
mod sentinel;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use sea_query::{Value, Values};
use sea_query_sqlx::SqlxValues;

use super::{Layout, PreparedQuery, SqlCtx};
use crate::aql::error::AqlError;
use crate::aql::ir::{Params, QueryIr};

use key::Key;
use live::Live;
use sentinel::Sentinels;

/// The keys a plan remembers, shaped or not.
///
/// A key is shaped on its second sighting, so a query that names a different
/// shape on every request costs one key comparison per remembered key and no
/// extra build. No openEHR spec governs this bound: our own tuning.
const KEYS_PER_PLAN: usize = 32;

/// The statement shapes of one plan.
pub struct Shapes {
    /// The plan's `$parameter`s that bind only an EHR id
    /// ([`crate::aql::ehr_id_params`]), which a slot carries.
    live_params: Vec<String>,
    entries: RwLock<Ring>,
    /// The requests bound from a shape.
    bound: AtomicU64,
}

/// The remembered keys, the oldest replaced first.
#[derive(Debug, Default)]
struct Ring {
    entries: Vec<Arc<Entry>>,
    next: usize,
}

/// A remembered key and what is known of its shape.
#[derive(Debug)]
struct Entry {
    key: Key,
    state: State,
}

/// What is known of the shape of a key.
#[derive(Debug)]
enum State {
    /// Seen once: the next request of the key derives its shape.
    Seen,
    /// Derived.
    Shaped(Shape),
    /// Its text or a value depends on a [`Live`] source in a way a slot
    /// cannot carry, so every request of the key is built.
    Unshaped,
}

/// One statement shape: the text, a slot per bound value, and the read-back
/// layout.
#[derive(Debug)]
struct Shape {
    sql: Arc<str>,
    slots: Vec<Slot>,
    layout: Arc<Layout>,
}

/// Where a bound value of a shape comes from.
#[derive(Debug)]
enum Slot {
    /// A value every request of the shape binds.
    Fixed(Value),
    /// A value read from the request.
    Live(Live),
}

// A key holds the parameter values and the condition of a request, so the
// shapes print only how many keys they remember.
impl std::fmt::Debug for Shapes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys = self
            .entries
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .entries
            .len();
        f.debug_struct("Shapes")
            .field("live_params", &self.live_params.len())
            .field("keys", &keys)
            .field("bound", &self.bound())
            .finish()
    }
}

impl Shapes {
    /// The shapes of `ir`, none known yet.
    #[must_use]
    pub fn new(ir: &QueryIr) -> Self {
        Self {
            live_params: crate::aql::ehr_id_params(ir),
            entries: RwLock::new(Ring::default()),
            bound: AtomicU64::new(0),
        }
    }

    /// How many requests were bound from a shape rather than built.
    #[must_use]
    pub fn bound(&self) -> u64 {
        self.bound.load(Ordering::Relaxed)
    }

    /// The statement of `ir` for `params` and `ctx`: bound from a shape when
    /// the request matches one, built otherwise.
    ///
    /// # Errors
    ///
    /// As [`super::build`].
    pub fn prepare(
        &self,
        ir: &QueryIr,
        params: &Params,
        ctx: &SqlCtx,
    ) -> Result<PreparedQuery, AqlError> {
        let person_withheld = crate::system_log::access_context::access_delay_enforced();
        let found = self.find(ir, params, ctx, person_withheld);
        if let Some(Entry {
            state: State::Shaped(shape),
            ..
        }) = found.as_deref()
            && let Some(prepared) = shape.fill(&self.live_params, params, ctx)
        {
            self.bound.fetch_add(1, Ordering::Relaxed);
            #[cfg(debug_assertions)]
            same_as_built(&prepared, &super::build(ir, params, ctx)?);
            return Ok(prepared);
        }
        let prepared = super::build(ir, params, ctx)?;
        match found {
            Some(entry) if matches!(entry.state, State::Seen) => {
                let state = self
                    .derive(ir, params, ctx, &prepared)
                    .map_or(State::Unshaped, State::Shaped);
                self.settle(&entry, state);
            }
            Some(_) => {}
            None => self.remember(Key::of(ir, &self.live_params, params, ctx, person_withheld)),
        }
        Ok(prepared)
    }

    /// The remembered entry whose key the request matches.
    fn find(
        &self,
        ir: &QueryIr,
        params: &Params,
        ctx: &SqlCtx,
        person_withheld: bool,
    ) -> Option<Arc<Entry>> {
        let ring = self.entries.read().unwrap_or_else(PoisonError::into_inner);
        ring.entries
            .iter()
            .find(|entry| {
                entry
                    .key
                    .admits(ir, &self.live_params, params, ctx, person_withheld)
            })
            .map(Arc::clone)
    }

    /// Remembers `key` as seen, unless a concurrent request did.
    fn remember(&self, key: Key) {
        let mut ring = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if ring.entries.iter().any(|entry| entry.key == key) {
            return;
        }
        ring.push(Arc::new(Entry {
            key,
            state: State::Seen,
        }));
    }

    /// Records what the second request of `entry`'s key found its shape to be.
    fn settle(&self, entry: &Arc<Entry>, state: State) {
        let mut ring = self.entries.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = ring
            .entries
            .iter_mut()
            .find(|held| Arc::ptr_eq(held, entry))
        {
            *held = Arc::new(Entry {
                key: entry.key.clone(),
                state,
            });
        }
    }

    /// The shape of the request `prepared` was built for, or `None` when its
    /// text or a value depends on a [`Live`] source in a way a slot cannot
    /// carry.
    fn derive(
        &self,
        ir: &QueryIr,
        params: &Params,
        ctx: &SqlCtx,
        prepared: &PreparedQuery,
    ) -> Option<Shape> {
        let (one, two) = (Sentinels::draw(1), Sentinels::draw(2));
        let (params_one, ctx_one) = one.mask(&self.live_params, params, ctx);
        let (params_two, ctx_two) = two.mask(&self.live_params, params, ctx);
        // NOTE: our own design (#5783), a stand-in the build refuses leaves the key unshaped.
        let first = super::build(ir, &params_one, &ctx_one).ok()?;
        let second = super::build(ir, &params_two, &ctx_two).ok()?;
        if first.sql != second.sql || first.sql != prepared.sql {
            return None;
        }
        let lives = Live::every(self.live_params.len());
        let images = |params: &Params, ctx: &SqlCtx| -> Vec<(Live, Value)> {
            lives
                .iter()
                .filter_map(|live| Some((*live, live.eval(&self.live_params, params, ctx)?)))
                .collect()
        };
        let (images_one, images_two) =
            (images(&params_one, &ctx_one), images(&params_two, &ctx_two));
        let first_values = &first.values.0.0;
        let second_values = &second.values.0.0;
        if first_values.len() != second_values.len() {
            return None;
        }
        let slots = first_values
            .iter()
            .zip(second_values)
            .map(|(a, b)| slot(a, b, &images_one, &images_two))
            .collect::<Option<Vec<Slot>>>()?;
        let shape = Shape {
            sql: Arc::clone(&prepared.sql),
            slots,
            layout: Arc::clone(&prepared.layout),
        };
        // NOTE: our own design (#5783), a shape is kept only when it binds what the build bound.
        let refilled = shape.fill(&self.live_params, params, ctx)?;
        (refilled.values.0.0 == prepared.values.0.0).then_some(shape)
    }
}

/// The slot of one bound value, seen as `a` in the first sentinel build and
/// `b` in the second; `None` when it is neither fixed nor one source's image.
fn slot(a: &Value, b: &Value, one: &[(Live, Value)], two: &[(Live, Value)]) -> Option<Slot> {
    let image = |images: &[(Live, Value)], value: &Value| {
        images
            .iter()
            .find(|(_, image)| image == value)
            .map(|(live, _)| *live)
    };
    match (image(one, a), image(two, b)) {
        (None, None) if a == b => Some(Slot::Fixed(a.clone())),
        (Some(first), Some(second)) if first == second => Some(Slot::Live(first)),
        _ => None,
    }
}

/// Holds a statement bound from a shape to the request's own build, in a
/// debug build, so every test that repeats a query checks its shapes.
#[cfg(debug_assertions)]
fn same_as_built(bound: &PreparedQuery, built: &PreparedQuery) {
    debug_assert_eq!(bound.sql, built.sql, "a shape's text is the build's");
    debug_assert!(
        bound.values.0.0 == built.values.0.0,
        "a shape binds the build's values"
    );
}

impl Ring {
    /// Remembers `entry`; when the ring is full it replaces the oldest key
    /// with no shape, or the oldest key when every key has one, so a stream
    /// of keys seen once does not push out the shapes in use.
    fn push(&mut self, entry: Arc<Entry>) {
        if self.entries.len() < KEYS_PER_PLAN {
            self.entries.push(entry);
            return;
        }
        let shapeless = (0..KEYS_PER_PLAN)
            .map(|step| (self.next + step) % KEYS_PER_PLAN)
            .find(|at| {
                self.entries
                    .get(*at)
                    .is_some_and(|held| !matches!(held.state, State::Shaped(_)))
            });
        let at = shapeless.unwrap_or(self.next);
        if let Some(held) = self.entries.get_mut(at) {
            *held = entry;
        }
        self.next = (at + 1) % KEYS_PER_PLAN;
    }
}

impl Shape {
    /// The statement of this shape bound from the request; `None` when a
    /// slot's source is absent from it, which a matched key rules out.
    fn fill(&self, live_params: &[String], params: &Params, ctx: &SqlCtx) -> Option<PreparedQuery> {
        let values = self
            .slots
            .iter()
            .map(|slot| match slot {
                Slot::Fixed(value) => Some(value.clone()),
                Slot::Live(live) => live.eval(live_params, params, ctx),
            })
            .collect::<Option<Vec<Value>>>()?;
        Some(PreparedQuery {
            sql: Arc::clone(&self.sql),
            values: SqlxValues(Values(values)),
            layout: Arc::clone(&self.layout),
        })
    }
}

#[cfg(test)]
mod tests;
