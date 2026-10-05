// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared UI components used by several screens — the design-system kit.
//!
//! Screens compose these instead of hand-rolling markup, so one affordance looks
//! and behaves the same wherever it appears. `wire` holds the two flattened CDR
//! shapes the kit itself renders, so no component reaches into a screen for a
//! type.

pub mod activity_chart;
pub mod brand;
pub mod confirm_dialog;
pub mod data_table;
pub mod empty_state;
pub mod example_controls;
pub mod facts;
pub mod field;
pub mod format_view;
pub mod item_tags;
pub mod logical_delete;
pub mod notice;
pub mod page_header;
pub mod results_chart;
pub mod scope_grants;
pub mod stat_card;
pub mod surface;
pub mod tab_bar;
pub mod toast;
pub mod upload_dialog;
pub mod version_history;
pub mod wire;
