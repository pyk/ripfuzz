//! Solidity source map parsing and types.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[cfg(test)]
use std::path::Path;

use alloy_primitives::keccak256;

use crate::contract::ContractId;
use crate::contract::artifact::ContractArtifact;
use crate::coverage::CoverageMap;

/// Parsed source map for a contract's bytecode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    /// Raw entries, one per instruction.
    pub entries: Vec<SourceMapEntry>,
    /// The source file path.
    pub source_path: PathBuf,
    /// Contract name.
    pub contract_name: String,
}

impl SourceMap {
    /// Parse a raw source map string into a [`SourceMap`].
    ///
    /// `source_path` and `contract_name` must be set by the caller after
    /// construction; they are not present in the raw string.
    pub fn parse(raw: &str) -> Self {
        Self {
            entries: parse_entries(raw),
            source_path: PathBuf::new(),
            contract_name: String::new(),
        }
    }

    /// Look up the source map entry for a given program counter.
    pub fn entry_for_pc(&self, pc: usize) -> Option<&SourceMapEntry> {
        self.entries.get(pc)
    }
}

/// A single entry in a Solidity source map.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceMapEntry {
    pub source_offset: usize,
    pub source_length: usize,
    pub source_file_index: usize,
    pub jump_type: JumpType,
    pub modifier_depth: usize,
}

/// Jump type for a source map entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JumpType {
    #[default]
    Regular,
    Into,
    Out,
}

/// A single hit resolved to a source location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceHit {
    pub source_path: PathBuf,
    pub source_offset: usize,
    pub source_length: usize,
    pub line_start: usize,
    pub column_start: usize,
    pub line_end: usize,
    pub column_end: usize,
    pub bucket: u8,
}

/// Coverage report with all hits resolved to source locations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceCoverageReport {
    pub hits: Vec<SourceHit>,
}

impl SourceCoverageReport {
    /// Total number of unique source locations hit.
    pub fn hit_count(&self) -> usize {
        self.hits.len()
    }
}

/// Key for deduplicating source locations during report generation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct SourceLocationKey {
    path: PathBuf,
    offset: usize,
    length: usize,
}

/// Resolve a coverage map to source-level hits using the artifact's source maps.
///
/// For each contract in the coverage map, looks up the corresponding source map
/// and converts every hit PC into a [`SourceHit`] with line and column numbers.
/// Duplicate source locations (multiple PCs mapping to the same source range)
/// are deduplicated, keeping the first encountered bucket value.
pub fn resolve_coverage_to_source(
    coverage: &CoverageMap,
    artifact: &ContractArtifact,
) -> SourceCoverageReport {
    let mut report = SourceCoverageReport::default();
    let mut seen: HashSet<SourceLocationKey> = HashSet::new();

    let runtime_id = ContractId::from(artifact.runtime.hash_slow());
    let init_id = ContractId::from(keccak256(&artifact.initcode));

    if let Some(contract_cov) = coverage.contracts.get(&runtime_id)
        && let Some(source_map) = &artifact.runtime_source_map
    {
        resolve_contract_coverage(contract_cov, source_map, &mut report, &mut seen);
    }

    if let Some(contract_cov) = coverage.contracts.get(&init_id)
        && let Some(source_map) = &artifact.init_source_map
    {
        resolve_contract_coverage(contract_cov, source_map, &mut report, &mut seen);
    }

    report
}

fn resolve_contract_coverage(
    contract_cov: &crate::coverage::ContractCoverage,
    source_map: &SourceMap,
    report: &mut SourceCoverageReport,
    seen: &mut HashSet<SourceLocationKey>,
) {
    let Ok(source_text) = fs::read_to_string(&source_map.source_path) else {
        return;
    };

    for (pc, bucket) in contract_cov.edges.iter().enumerate() {
        if *bucket == 0 {
            continue;
        }
        let Some(entry) = source_map.entries.get(pc) else {
            continue;
        };

        let key = SourceLocationKey {
            path: source_map.source_path.clone(),
            offset: entry.source_offset,
            length: entry.source_length,
        };
        if !seen.insert(key) {
            continue;
        }

        let (line_start, column_start) = offset_to_line_col(&source_text, entry.source_offset);
        let end_offset = entry.source_offset.saturating_add(entry.source_length);
        let (line_end, column_end) = offset_to_line_col(&source_text, end_offset);

        report.hits.push(SourceHit {
            source_path: source_map.source_path.clone(),
            source_offset: entry.source_offset,
            source_length: entry.source_length,
            line_start,
            column_start,
            line_end,
            column_end,
            bucket: *bucket,
        });
    }
}

fn offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for (i, c) in source.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn parse_entries(raw: &str) -> Vec<SourceMapEntry> {
    let mut entries = Vec::new();
    let mut prev = SourceMapEntry::default();

    if raw.is_empty() {
        return entries;
    }

    for entry in raw.split(';') {
        let parts: Vec<&str> = entry.split(':').collect();
        let s = parse_field(parts.first(), prev.source_offset);
        let l = parse_field(parts.get(1), prev.source_length);
        let f = parse_field(parts.get(2), prev.source_file_index);
        let j = parse_jump(parts.get(3).copied().unwrap_or(""));
        let m = parse_field(parts.get(4), prev.modifier_depth);

        let new = SourceMapEntry {
            source_offset: s,
            source_length: l,
            source_file_index: f,
            jump_type: j,
            modifier_depth: m,
        };
        entries.push(new);
        prev = new;
    }

    entries
}

fn parse_field(opt: Option<&&str>, default: usize) -> usize {
    match opt {
        Some(s) if !s.is_empty() => s.parse().unwrap_or(default),
        _ => default,
    }
}

fn parse_jump(s: &str) -> JumpType {
    match s {
        "i" => JumpType::Into,
        "o" => JumpType::Out,
        "-" | "" => JumpType::Regular,
        _ => JumpType::Regular,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_source_map() {
        let raw = "137:88:0:-:0;201:3:0:-:0;";
        let map = SourceMap::parse(raw);
        assert_eq!(map.entries.len(), 3);
        assert_eq!(map.entries[0].source_offset, 137);
        assert_eq!(map.entries[0].source_length, 88);
        assert_eq!(map.entries[0].source_file_index, 0);
        assert_eq!(map.entries[0].jump_type, JumpType::Regular);
        assert_eq!(map.entries[0].modifier_depth, 0);

        assert_eq!(map.entries[1].source_offset, 201);
        assert_eq!(map.entries[1].source_length, 3);
        assert_eq!(map.entries[1].modifier_depth, 0);

        // Third entry is empty, so inherits from previous.
        assert_eq!(map.entries[2].source_offset, 201);
        assert_eq!(map.entries[2].source_length, 3);
    }

    #[test]
    fn parse_empty_string() {
        let map = SourceMap::parse("");
        assert!(map.entries.is_empty());
    }

    #[test]
    fn resolve_coverage_maps_pc_to_source_location() {
        let artifact = crate::contract::ContractBuilder::build(
            Path::new("fixtures/basic-target"),
            Path::new("test/Target.sol"),
        )
        .unwrap();

        assert!(
            artifact.runtime_source_map.is_some(),
            "runtime source map must be present"
        );
        assert!(
            !artifact
                .runtime_source_map
                .as_ref()
                .unwrap()
                .entries
                .is_empty(),
            "runtime source map must have entries"
        );

        let mut coverage = CoverageMap::default();
        let runtime_id = ContractId::from(artifact.runtime.hash_slow());
        let mut contract_cov = crate::coverage::ContractCoverage::new(artifact.runtime.len());
        contract_cov.edges[0] = 1;
        coverage.contracts.insert(runtime_id, contract_cov);

        let report = resolve_coverage_to_source(&coverage, &artifact);
        assert!(
            !report.hits.is_empty(),
            "should resolve at least one source hit"
        );

        let hit = &report.hits[0];
        assert!(hit.line_start > 0);
        assert!(hit.column_start > 0);
        assert!(!hit.source_path.as_os_str().is_empty());
    }
}
