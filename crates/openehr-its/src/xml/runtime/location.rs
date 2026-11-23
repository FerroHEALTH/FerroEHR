// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Where a refusal of the canonical-XML reader points: the element it
//! describes, resolved to its line, column and path from the root.

use super::{Location, XmlReader};

impl XmlReader<'_> {
    /// The element a construction failure describes: the one closed most
    /// recently, because a `FromXml` impl builds its value right after it has
    /// consumed its own end tag. Before any element closed, the document.
    pub(super) fn located(&self) -> Location {
        let Some(closed) = &self.last_closed else {
            return Location {
                element: "document".to_owned(),
                xsi_type: None,
                line: 1,
                column: 1,
                path: "/".to_owned(),
            };
        };
        self.location_of(
            &closed.element,
            closed.xsi_type.as_ref(),
            closed.offset,
            closed.ordinal,
            closed.ancestors,
        )
    }

    /// The element the reader opened most recently and has not closed, which
    /// a check on a child's start tag describes; the document before any
    /// element opened.
    pub(super) fn located_open(&self) -> Location {
        let Some((open, ancestors)) = self.frames.split_last() else {
            return self.located();
        };
        self.location_of(
            &open.element,
            open.xsi_type.as_ref(),
            open.offset,
            open.ordinal,
            ancestors.len(),
        )
    }

    /// The [`Location`] of an element under the first `ancestors` open frames.
    fn location_of(
        &self,
        element: &str,
        xsi_type: Option<&String>,
        offset: usize,
        ordinal: u32,
        ancestors: usize,
    ) -> Location {
        let mut path = String::new();
        for (i, frame) in self.frames.iter().take(ancestors).enumerate() {
            path.push('/');
            path.push_str(&frame.element);
            if i > 0 {
                path.push('[');
                path.push_str(&frame.ordinal.to_string());
                path.push(']');
            }
        }
        path.push('/');
        path.push_str(element);
        if ancestors > 0 {
            path.push('[');
            path.push_str(&ordinal.to_string());
            path.push(']');
        }
        let (line, column) = line_col(self.src, offset);
        Location {
            element: element.to_owned(),
            xsi_type: xsi_type.cloned(),
            line,
            column,
            path,
        }
    }
}

/// The 1-based line and column of byte `offset` in `src`; the column counts
/// characters.
fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let head = src.get(..offset.min(src.len())).unwrap_or(src);
    let line = head.matches('\n').count() + 1;
    let column = head.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}
