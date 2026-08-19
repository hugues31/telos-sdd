//! Byte-offset source spans, used to attach positions to referential leaves
//! of the parsed AST (identifiers referencing notions, fields, ids...).

/// A byte-offset range `[start, end)` into a source file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// A node paired with the span it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sp<T> {
    pub node: T,
    pub span: Span,
}

/// Converts a byte `offset` into `src` to a 1-indexed `(line, col)` pair.
///
/// Both line and column are 1-indexed. `offset` is a byte offset, matching
/// `Span`'s byte-offset semantics.
pub fn line_col(src: &str, offset: u32) -> (u32, u32) {
    let offset = offset as usize;
    let mut line: u32 = 1;
    let mut line_start: usize = 0;
    for (i, b) in src.bytes().enumerate().take(offset) {
        if b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let col = (offset - line_start) as u32 + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_finds_second_line_first_column() {
        assert_eq!(line_col("ab\ncd", 3), (2, 1));
    }

    #[test]
    fn line_col_at_offset_zero_is_first_line_first_column() {
        assert_eq!(line_col("hello", 0), (1, 1));
    }

    #[test]
    fn line_col_counts_multiple_newlines() {
        assert_eq!(line_col("a\nb\nc", 4), (3, 1));
    }

    #[test]
    fn line_col_mid_line_advances_column() {
        assert_eq!(line_col("hello world", 6), (1, 7));
    }

    #[test]
    fn span_default_is_zero_length_at_origin() {
        assert_eq!(Span::default(), Span { start: 0, end: 0 });
    }

    #[test]
    fn sp_wraps_a_node_with_its_span() {
        let sp = Sp {
            node: "Invoice",
            span: Span { start: 5, end: 12 },
        };
        assert_eq!(sp.node, "Invoice");
        assert_eq!(sp.span, Span { start: 5, end: 12 });
    }
}
