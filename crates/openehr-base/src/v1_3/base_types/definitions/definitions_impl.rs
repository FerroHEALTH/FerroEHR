// @generated-from-template templates/openehr-base/base_types/definitions/definitions_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written realization of the BASE `definitions` constant-holder classes
//! — `BASIC_DEFINITIONS` and `OPENEHR_DEFINITIONS` (which inherits it, adding
//! `Local_terminology_id`).
//!
//! Both are adjudicated out of type emission (constant holders, no data — the
//! codegen decision map), so their spec constants live here as plain module
//! constants.
//!
//! Spec sources (vendored):
//! - `BASE/docs/UML/classes/org.openehr.base.base_types.basic_definitions.adoc`
//!   (§Constants: `CR`, `LF`, `Any_type_name`, `Regex_any_pattern`,
//!   `Default_encoding`, `None_type_name`).
//! - `BASE/docs/UML/classes/org.openehr.base.base_types.openehr_definitions.adoc`
//!   (§Constants: `Local_terminology_id` = "local" — "Predefined terminology
//!   identifier to indicate it is local to the knowledge resource in which it
//!   occurs, e.g. an archetype").

/// `BASIC_DEFINITIONS.CR` — carriage return character (`'\015'`).
pub const CR: char = '\u{000D}';

/// `BASIC_DEFINITIONS.LF` — line feed character (`'\012'`).
pub const LF: char = '\u{000A}';

/// `BASIC_DEFINITIONS.Any_type_name` — the name of the universal `Any` type.
pub const ANY_TYPE_NAME: &str = "Any";

/// `BASIC_DEFINITIONS.Regex_any_pattern` — the match-anything regex.
pub const REGEX_ANY_PATTERN: &str = ".*";

/// `BASIC_DEFINITIONS.Default_encoding` — the assumed string encoding
/// (foundation_types master03 §Unicode: UTF-8 assumed).
pub const DEFAULT_ENCODING: &str = "UTF-8";

/// `BASIC_DEFINITIONS.None_type_name` — the name of the `None` type.
pub const NONE_TYPE_NAME: &str = "None";

/// `OPENEHR_DEFINITIONS.Local_terminology_id` — the predefined terminology
/// identifier meaning "local to the knowledge resource in which it occurs,
/// e.g. an archetype".
pub const LOCAL_TERMINOLOGY_ID: &str = "local";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_carry_the_spec_values() {
        // basic_definitions.adoc §Constants: octal '\015' / '\012'.
        assert_eq!(u32::from(CR), 0o15);
        assert_eq!(u32::from(LF), 0o12);
        assert_eq!(ANY_TYPE_NAME, "Any");
        assert_eq!(REGEX_ANY_PATTERN, ".*");
        assert_eq!(DEFAULT_ENCODING, "UTF-8");
        assert_eq!(NONE_TYPE_NAME, "None");
        // openehr_definitions.adoc §Constants.
        assert_eq!(LOCAL_TERMINOLOGY_ID, "local");
    }
}
