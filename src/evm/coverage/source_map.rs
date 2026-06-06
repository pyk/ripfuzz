//! Solidity source map parsing.

/// A single entry in a Solidity source map.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceMapEntry {
    /// Byte offset in the source file.
    pub offset: usize,
    /// Length of the source range in bytes.
    pub length: usize,
    /// Index of the source file (`-1` means no source).
    pub source_index: isize,
    /// Jump type (`i`, `o`, `-`).
    pub jump_type: char,
    /// Modifier depth.
    pub modifier_depth: usize,
}

/// Parse a Solidity source map string into a list of entries.
///
/// The source map format is a semicolon-separated list of `s:l:f:j:m`
/// entries. Missing values reuse the previous entry's value.
pub fn parse_source_map(source_map: &str) -> Vec<SourceMapEntry> {
    let mut entries = Vec::new();
    if source_map.is_empty() {
        return entries;
    }
    let mut current = SourceMapEntry::default();

    for segment in source_map.split(';') {
        if segment.is_empty() {
            entries.push(current);
            continue;
        }
        let parts: Vec<&str> = segment.split(':').collect();
        if !parts.is_empty() && !parts[0].is_empty() {
            current.offset = parts[0].parse().unwrap_or(0);
        }
        if parts.len() > 1 && !parts[1].is_empty() {
            current.length = parts[1].parse().unwrap_or(0);
        }
        if parts.len() > 2 && !parts[2].is_empty() {
            current.source_index = parts[2].parse().unwrap_or(0);
        }
        if parts.len() > 3 && !parts[3].is_empty() {
            current.jump_type = parts[3].chars().next().unwrap_or('-');
        }
        if parts.len() > 4 && !parts[4].is_empty() {
            current.modifier_depth = parts[4].parse().unwrap_or(0);
        }
        entries.push(current);
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_source_map() {
        let entries = parse_source_map("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_simple_source_map() {
        let entries = parse_source_map("0:1:2:i:3;4:5:6:o:7");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].offset, 0);
        assert_eq!(entries[0].length, 1);
        assert_eq!(entries[0].source_index, 2);
        assert_eq!(entries[0].jump_type, 'i');
        assert_eq!(entries[0].modifier_depth, 3);
        assert_eq!(entries[1].offset, 4);
        assert_eq!(entries[1].length, 5);
        assert_eq!(entries[1].source_index, 6);
        assert_eq!(entries[1].jump_type, 'o');
        assert_eq!(entries[1].modifier_depth, 7);
    }

    #[test]
    fn parse_no_source() {
        let entries = parse_source_map("0:0:-1:-:0");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].source_index, -1);
    }

    #[test]
    fn parse_source_map_with_reuse() {
        let entries = parse_source_map("0:1:2:i:3;;");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[1].offset, 0);
        assert_eq!(entries[2].offset, 0);
    }
}
