// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The macro that defines one group of the draft access terminology.

/// Defines one group's term type, its [`Term`](super::Term) impl and its
/// `Display`.
///
/// A term is `Name => "code"`, a code of
/// [`TERMINOLOGY_ID_ACCESS`](super::TERMINOLOGY_ID_ACCESS) that is
/// its own rubric, or `Name => "code" of TERMINOLOGY, "rubric"`, the code of
/// another terminology.
macro_rules! group {
    (@terminology) => { TERMINOLOGY_ID_ACCESS };
    (@terminology $terminology:ident) => { $terminology };
    (@rubric $code:literal) => { $code };
    (@rubric $code:literal $rubric:literal) => { $rubric };
    (
        $(#[$doc:meta])*
        $name:ident in $group:ident {
            $( $(#[$term_doc:meta])* $term:ident => $code:literal $(of $terminology:ident, $rubric:literal)?, )+
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $( $(#[$term_doc])* $term, )+
        }

        impl Term for $name {
            const GROUP: Group = Group::$group;
            const ALL: &'static [Self] = &[$(Self::$term),+];

            fn terminology(self) -> &'static str {
                match self {
                    $(Self::$term => group!(@terminology $($terminology)?),)+
                }
            }

            fn code(self) -> &'static str {
                match self {
                    $(Self::$term => $code,)+
                }
            }

            fn rubric(self) -> &'static str {
                match self {
                    $(Self::$term => group!(@rubric $code $($rubric)?),)+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.code())
            }
        }
    };
}
