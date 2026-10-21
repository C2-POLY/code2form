/// A byte range in one source file. File 0 is the file being parsed;
/// included files get the ids their [`Includes`](crate::parser::Includes)
/// resolver hands out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub file: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start: start as u32, end: end as u32, file: 0 }
    }

    pub fn in_file(start: usize, end: usize, file: u32) -> Self {
        Span { start: start as u32, end: end as u32, file }
    }

    /// From this span to `other`; a span reaching into another file stops
    /// at this one.
    pub fn to(self, other: Span) -> Span {
        if other.file != self.file {
            return self;
        }
        Span { start: self.start.min(other.start), end: self.end.max(other.end), file: self.file }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.start as usize..self.end as usize
    }
}

/// Maps byte offsets to zero-based line numbers.
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        LineIndex { starts }
    }

    /// Zero-based line containing `offset`.
    pub fn line(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset) - 1
    }

    /// Zero-based (line, column in chars) of `offset`.
    pub fn line_col(&self, src: &str, offset: usize) -> (usize, usize) {
        let line = self.line(offset);
        let col = src[self.starts[line]..offset].chars().count();
        (line, col)
    }

    /// Byte offset where `line` starts.
    pub fn line_start(&self, line: usize) -> usize {
        self.starts[line]
    }

    /// The line without its `\n` or `\r\n` terminator.
    pub fn line_text<'a>(&self, src: &'a str, line: usize) -> &'a str {
        let raw = self.raw_line_text(src, line);
        raw.strip_suffix('\r').unwrap_or(raw)
    }

    /// The line without its `\n`, keeping a `\r` before it.
    pub fn raw_line_text<'a>(&self, src: &'a str, line: usize) -> &'a str {
        let start = self.starts[line];
        let end = self.starts.get(line + 1).map_or(src.len(), |&e| e - 1);
        &src[start..end]
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }
}
