// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Web Template model and builder (`simplified_formats
//! master04-basic_concepts.adoc` §"Web Template Metadata").

mod binding_v2_4;
pub mod builder;
pub mod builder_v2_4;
mod cardinality_v2_4;
pub mod constraint;
mod existence;
pub mod facts;
mod id;
mod implicit;
mod in_context;
mod inputs;
mod interval_v2_4;
pub mod model;
mod path_v2_4;
mod root;
mod shape;

pub(crate) use inputs::PROPORTION_KINDS;
