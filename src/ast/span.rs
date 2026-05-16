//! Byte-offset spans used as the default user-data type for
//! [`super::SpecFile`] when the parser is invoked with span tracking.

/// Half-open range `[start_byte, end_byte)` into the original source string,
/// annotated with 1-based line and column at both ends.
///
/// Invariant: `start_byte <= end_byte`. Construction via [`Span::new`]
/// asserts this in debug builds. Methods that assume the invariant likewise
/// document their reliance on it.
///
/// Lines and columns are 1-based, consistent with `nom_locate`. Lines count
/// `\n` boundaries; columns are byte offsets within the line. Multi-byte
/// UTF-8 codepoints occupy several columns by this convention — consumers
/// that need character-based columns should convert at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct Span {
    /// Inclusive byte offset of the first byte covered by this span.
    pub start_byte: usize,
    /// Exclusive byte offset just past the last byte covered.
    pub end_byte: usize,
    /// 1-based line number where the span starts.
    pub start_line: u32,
    /// 1-based byte column on `start_line` where the span starts.
    pub start_column: u32,
    /// 1-based line number where the span ends.
    pub end_line: u32,
    /// 1-based byte column on `end_line` where the span ends.
    pub end_column: u32,
}

impl Span {
    /// Build a new span. Panics in debug if `start_byte > end_byte`.
    #[must_use]
    pub const fn new(
        start_byte: usize,
        end_byte: usize,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    ) -> Self {
        debug_assert!(
            start_byte <= end_byte,
            "Span::new: start_byte must be <= end_byte"
        );
        Self {
            start_byte,
            end_byte,
            start_line,
            start_column,
            end_line,
            end_column,
        }
    }

    /// Build a span that covers only a byte range, leaving line/column zero.
    /// Useful in tests and for callers that do not track lines.
    #[must_use]
    pub const fn from_bytes(start_byte: usize, end_byte: usize) -> Self {
        debug_assert!(
            start_byte <= end_byte,
            "Span::from_bytes: start_byte must be <= end_byte"
        );
        Self {
            start_byte,
            end_byte,
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
        }
    }

    /// Length in bytes. Relies on the `start_byte <= end_byte` invariant.
    #[must_use]
    pub const fn len(&self) -> usize {
        debug_assert!(
            self.start_byte <= self.end_byte,
            "Span::len: invariant violated"
        );
        self.end_byte - self.start_byte
    }

    /// Returns `true` when the span covers zero bytes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start_byte >= self.end_byte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_len() {
        let s = Span::new(5, 10, 1, 6, 1, 11);
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert_eq!(s.start_line, 1);
        assert_eq!(s.end_column, 11);
    }

    #[test]
    fn from_bytes_zeroes_lines() {
        let s = Span::from_bytes(2, 7);
        assert_eq!(s.len(), 5);
        assert_eq!(s.start_line, 0);
        assert_eq!(s.end_line, 0);
    }

    #[test]
    fn empty_span() {
        let s = Span::new(7, 7, 1, 8, 1, 8);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn default_is_zero() {
        let s = Span::default();
        assert_eq!(s.start_byte, 0);
        assert_eq!(s.end_byte, 0);
        assert!(s.is_empty());
    }

    #[test]
    #[should_panic(expected = "Span::new")]
    #[cfg(debug_assertions)]
    fn debug_reject_inverted() {
        let _ = Span::new(10, 5, 1, 11, 1, 6);
    }
}
