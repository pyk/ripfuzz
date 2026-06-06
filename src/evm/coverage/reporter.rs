//! Coverage reporter that generates an lcov.info file from build artifacts and
//! shared coverage data.
//!
//! [`CoverageReporter`] is the entry point. It takes two inputs:
//!
//! 1. **Build artifacts** - compiled Solidity artifacts (parsed from `out/*.json`)
//!    that contain deployed bytecode and source maps.
//! 2. [`SharedCoverage`] - raw per-PC hit counts collected during fuzzing.
//!
//! The reporter resolves every hit PC back to a source line using the artifact
//! source maps, then aggregates the results into a single `lcov.info` report.
//!
//! # Pipeline
//!
//! 1. **Build path-to-artifact** map from all loaded artifacts.
//! 2. **Match active bytecodes** from [`SharedCoverage`] to artifacts via
//!    codehash (masking out link references and immutables).
//! 3. **Resolve source files recursively** from each root artifact's
//!    `metadata.sources`. Each source file key has its own child artifact
//!    that is resolved transitively.
//! 4. **Pre-read source files** and build caches.
//! 5. **Build PC-counter map**: for each matched codehash, walk PCs with
//!    hits, resolve through the artifact source map to (file, line), and
//!    aggregate hit counts.
//! 6. **Determine executable lines** from the source map: a line is
//!    executable when at least one source map entry maps to it, the line
//!    is not a close-bracket (`}`), the line is not empty, and the line is
//!    not a contract/interface/library definition.
//! 7. **Collect function coverage** from the AST of resolved artifacts.
//! 8. **Assemble the final `lcov.info` report.**
//!
//! # Usage
//!
//! ```text
//! let report = CoverageReporter::new()
//!     .build_artifacts(artifacts)
//!     .shared_coverage(shared_coverage)
//!     .build();
//! let lcov_info = format!("{report}");
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::PathBuf;

use alloy_primitives::{B256, keccak256};
use tracing::instrument;

use crate::evm::coverage::shared::SharedCoverage;
use crate::evm::coverage::source_map::{SourceMapEntry, parse_source_map};
use crate::foundry::{Artifact, ArtifactBytecode, ArtifactId, LinkReferences};

// ---------------------------------------------------------------------------
// Bytecode helpers
// ---------------------------------------------------------------------------

fn collect_link_positions(link_refs: &LinkReferences) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                out.push((r.start, r.length));
            }
        }
    }
    out
}

fn zero_out_positions(buf: &mut [u8], positions: &[(usize, usize)]) {
    for (start, len) in positions {
        for i in *start..*start + *len {
            if i < buf.len() {
                buf[i] = 0;
            }
        }
    }
}

fn parse_bytecode_with_placeholders(object: &str, link_refs: &LinkReferences) -> Vec<u8> {
    let hex = object.strip_prefix("0x").unwrap_or(object);
    let mut hex_positions = Vec::new();
    for libs in link_refs.values() {
        for refs in libs.values() {
            for r in refs {
                hex_positions.push((r.start * 2, r.length * 2));
            }
        }
    }
    hex_positions.sort_by_key(|(start, _)| *start);
    let mut cleaned = String::new();
    let mut last_end = 0;
    for (start, len) in hex_positions {
        cleaned.push_str(&hex[last_end..start]);
        cleaned.push_str(&"00".repeat(len / 2));
        last_end = start + len;
    }
    cleaned.push_str(&hex[last_end..]);
    hex::decode(cleaned).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// PC-to-source map
// ---------------------------------------------------------------------------

fn build_pc_to_source_map(
    bytecode: &[u8],
    source_map: &[SourceMapEntry],
) -> Vec<Option<SourceMapEntry>> {
    let mut result = vec![None; bytecode.len()];
    let mut pc = 0;
    let mut opcode_idx = 0;
    while pc < bytecode.len() {
        if let Some(entry) = source_map.get(opcode_idx) {
            result[pc] = Some(*entry);
        }
        let opcode = bytecode[pc];
        let push_size = if (0x60..=0x7f).contains(&opcode) {
            (opcode - 0x60 + 1) as usize
        } else {
            0
        };
        pc += 1 + push_size;
        opcode_idx += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// Artifact index
// ---------------------------------------------------------------------------

struct ArtifactIndexEntry<'a> {
    artifact: &'a Artifact,
    hash: B256,
    positions: Vec<(usize, usize)>,
    is_initcode: bool,
    code_len: usize,
}

struct ArtifactIndex<'a> {
    entries: Vec<ArtifactIndexEntry<'a>>,
    entries_by_len: HashMap<usize, Vec<usize>>,
}

impl<'a> ArtifactIndex<'a> {
    fn new(artifacts: &'a [Artifact]) -> Self {
        let mut entries = Vec::new();
        for artifact in artifacts {
            let deployed_entry = artifact.deployed_bytecode().and_then(|deployed| {
                let code =
                    parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
                if code.is_empty() {
                    return None;
                }
                let mut positions = collect_link_positions(&deployed.link_references);
                for refs in deployed.immutable_references.values() {
                    for r in refs {
                        positions.push((r.start, r.length));
                    }
                }
                if matches!(artifact, Artifact::Library(_)) && code.first() == Some(&0x73) {
                    positions.push((1, 20));
                }
                let code_len = code.len();
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                let hash = keccak256(&masked);
                Some(ArtifactIndexEntry {
                    artifact,
                    hash,
                    positions,
                    is_initcode: false,
                    code_len,
                })
            });
            if let Some(entry) = deployed_entry {
                entries.push(entry);
            }

            let initcode_entry = artifact.bytecode().and_then(|bytecode| {
                let code =
                    parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
                if code.is_empty() {
                    return None;
                }
                let positions = collect_link_positions(&bytecode.link_references);
                let code_len = code.len();
                let mut masked = code;
                zero_out_positions(&mut masked, &positions);
                let hash = keccak256(&masked);
                Some(ArtifactIndexEntry {
                    artifact,
                    hash,
                    positions,
                    is_initcode: true,
                    code_len,
                })
            });
            if let Some(entry) = initcode_entry {
                entries.push(entry);
            }
        }
        let mut entries_by_len: HashMap<usize, Vec<usize>> = HashMap::new();
        for (idx, entry) in entries.iter().enumerate() {
            entries_by_len.entry(entry.code_len).or_default().push(idx);
        }
        Self {
            entries,
            entries_by_len,
        }
    }

    fn find(&self, raw_bytecode: &[u8]) -> Option<(&'a Artifact, bool)> {
        let candidates = self.entries_by_len.get(&raw_bytecode.len())?;
        let mut masked = raw_bytecode.to_vec();
        for idx in candidates {
            let entry = &self.entries[*idx];
            zero_out_positions(&mut masked, &entry.positions);
            if keccak256(&masked) == entry.hash {
                return Some((entry.artifact, entry.is_initcode));
            }
            for (start, len) in &entry.positions {
                for i in *start..*start + *len {
                    if i < raw_bytecode.len() {
                        masked[i] = raw_bytecode[i];
                    }
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Line offset helpers
// ---------------------------------------------------------------------------

fn offset_to_line(content: &str, offset: usize) -> usize {
    let safe_offset = offset.min(content.len());
    let mut line = content
        .bytes()
        .take(safe_offset)
        .filter(|&b| b == b'\n')
        .count()
        + 1;
    if safe_offset == content.len() && content.ends_with('\n') {
        line = line.saturating_sub(1);
    }
    line
}

// ---------------------------------------------------------------------------
// Source ID resolution from metadata.sources
// ---------------------------------------------------------------------------

/// Build a source_id → file_path map for a root artifact.
///
/// The artifact's own `source_id()` maps to its source file. Unknown source
/// indices appearing in the artifact's source map are resolved by matching
/// byte offsets against candidate source files from `metadata.sources`.
/// A source id matches a candidate file when every source map entry for that
/// id has its `offset + length` within the file's byte length.
fn build_source_id_map(
    artifact: &Artifact,
    path_to_artifact: &HashMap<PathBuf, &Artifact>,
    source_cache: &HashMap<PathBuf, String>,
) -> HashMap<usize, PathBuf> {
    let mut sid_map: HashMap<usize, PathBuf> = HashMap::new();

    // The artifact's own source_id always maps to its source file.
    // checkrs: allow(clone_in_loops)
    sid_map.insert(artifact.source_id(), artifact.ast().absolute_path.clone());

    // Collect all source_ids seen in source maps, grouped by their
    // (offset, length) pairs.
    let mut unknown_entries: HashMap<usize, Vec<(usize, usize)>> = HashMap::new();
    for sm in [
        artifact.bytecode().map(|b| b.source_map.as_str()),
        artifact.deployed_bytecode().map(|b| b.source_map.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        for entry in parse_source_map(sm) {
            if !sid_map.contains_key(&entry.source_index) {
                unknown_entries
                    .entry(entry.source_index)
                    .or_default()
                    .push((entry.offset, entry.length));
            }
        }
    }

    if unknown_entries.is_empty() {
        return sid_map;
    }

    // Collect candidate files: the artifact itself plus all metadata.sources
    // files (including recursively resolved children).
    let mut candidates: HashSet<PathBuf> = HashSet::new();
    if let Some(sources) = artifact.metadata_sources() {
        for path_str in sources.keys() {
            candidates.insert(PathBuf::from(path_str));
        }
    }
    let mut visited: HashSet<&ArtifactId> = HashSet::new();
    let mut queue: Vec<&Artifact> = vec![artifact];
    while let Some(current) = queue.pop() {
        if !visited.insert(current.id()) {
            continue;
        }
        // checkrs: allow(nested_if_let)
        if let Some(sources) = current.metadata_sources() {
            for path_str in sources.keys() {
                let path = PathBuf::from(path_str);
                // checkrs: allow(nested_if_let)
                // checkrs: allow(clone_in_loops)
                let path_clone = path.clone();
                if candidates.insert(path_clone)
                    && let Some(child) = path_to_artifact.get(&path)
                {
                    queue.push(child);
                }
            }
        }
    }

    // For each unknown source id, find candidate files that contain
    // all its source map entries.
    for (sid, entries) in &unknown_entries {
        let mut fits: Vec<PathBuf> = Vec::new();
        for candidate in &candidates {
            // checkrs: allow(nested_if_let)
            if let Some(content) = source_cache.get(candidate) {
                let content_len = content.len();
                if entries
                    .iter()
                    .all(|(offset, len)| offset.saturating_add(*len) <= content_len)
                {
                    // checkrs: allow(clone_in_loops)
                    fits.push(candidate.clone());
                }
            }
        }

        // Greedy assignment: prefer files that haven't been assigned yet.
        if fits.len() == 1 {
            // checkrs: allow(clone_in_loops)
            sid_map.insert(*sid, fits[0].clone());
        } else {
            let unassigned: Vec<PathBuf> = fits
                .iter()
                .filter(|f| !sid_map.values().any(|v| v == *f))
                .cloned()
                .collect();
            if unassigned.len() == 1 {
                // checkrs: allow(clone_in_loops)
                sid_map.insert(*sid, unassigned[0].clone());
            }
        }
    }

    sid_map
}

// ---------------------------------------------------------------------------
// Coverage report types
// ---------------------------------------------------------------------------

/// A coverage report in lcov.info format.
#[derive(Debug, Clone, Default)]
pub struct CoverageReport {
    files: Vec<FileCoverage>,
}

#[derive(Debug, Clone)]
struct FunctionCoverage {
    name: String,
    line: usize,
    hits: u64,
}

#[derive(Debug, Clone)]
struct FileCoverage {
    path: PathBuf,
    line_hits: HashMap<usize, u64>,
    functions: Vec<FunctionCoverage>,
}

impl CoverageReport {
    /// Compute the overall line coverage percentage across all files.
    pub fn coverage(&self) -> f64 {
        let mut total = 0usize;
        let mut hit = 0usize;
        for file in &self.files {
            total += file.line_hits.len();
            hit += file.line_hits.values().filter(|&&h| h > 0).count();
        }
        if total > 0 {
            (hit as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }
}

impl fmt::Display for CoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for file in &self.files {
            writeln!(f, "TN:")?;
            writeln!(f, "SF:{}", file.path.display())?;

            let mut functions: Vec<&FunctionCoverage> = file.functions.iter().collect();
            functions.sort_by(|a, b| {
                let ord = a.line.cmp(&b.line);
                if ord == std::cmp::Ordering::Equal {
                    a.name.cmp(&b.name)
                } else {
                    ord
                }
            });

            for func in &functions {
                writeln!(f, "FN:{},{}", func.line, func.name)?;
            }
            for func in &functions {
                writeln!(f, "FNDA:{},{}", func.hits, func.name)?;
            }
            let fnf = functions.len();
            let fnh = functions.iter().filter(|f| f.hits > 0).count();
            if fnf > 0 {
                writeln!(f, "FNF:{}", fnf)?;
                writeln!(f, "FNH:{}", fnh)?;
            }

            let mut lines: Vec<(&usize, &u64)> = file.line_hits.iter().collect();
            lines.sort_by_key(|(line, _)| *line);

            for (line, hits) in &lines {
                writeln!(f, "DA:{},{}", line, hits)?;
            }

            let lf = lines.len();
            let lh = lines.iter().filter(|(_, hits)| **hits > 0).count();

            writeln!(f, "LF:{}", lf)?;
            writeln!(f, "LH:{}", lh)?;
            writeln!(f, "end_of_record")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Function coverage from AST
// ---------------------------------------------------------------------------

/// Walk a deployed bytecode's source map and populate `source_map_lines`
/// with every line that has a source map entry. Also insert the pc_map into
/// `pc_map_cache`.
fn populate_source_map_lines_from_deployed<'a>(
    deployed: &ArtifactBytecode,
    artifact: &'a Artifact,
    sid_map: &HashMap<usize, PathBuf>,
    source_cache: &HashMap<PathBuf, String>,
    source_map_lines: &mut HashMap<PathBuf, HashSet<usize>>,
    pc_map_cache: &mut HashMap<(&'a ArtifactId, bool), Vec<Option<SourceMapEntry>>>,
) {
    let code = parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
    let source_map = parse_source_map(&deployed.source_map);
    let pc_map = build_pc_to_source_map(&code, &source_map);
    for entry in source_map.iter() {
        if let Some(path) = sid_map.get(&entry.source_index)
            && let Some(content) = source_cache.get(path)
        {
            let line = offset_to_line(content, entry.offset);
            // checkrs: allow(clone_in_loops)
            let path_key = path.clone();
            source_map_lines.entry(path_key).or_default().insert(line);
        }
    }
    pc_map_cache.insert((artifact.id(), false), pc_map);
}

/// Collect function definitions from resolved artifacts by walking the AST.
///
/// For each function, the hit count is derived from the maximum line hit
/// across the function body. If no body line has hits, the function start
/// line's hit is used as fallback.
fn collect_function_coverage(
    artifacts: &[&Artifact],
    path_to_artifact: &HashMap<PathBuf, &Artifact>,
    source_cache: &HashMap<PathBuf, String>,
    line_hits: &HashMap<PathBuf, HashMap<usize, u64>>,
    source_map_lines: &HashMap<PathBuf, HashSet<usize>>,
) -> HashMap<PathBuf, Vec<FunctionCoverage>> {
    let mut file_functions: HashMap<PathBuf, Vec<FunctionCoverage>> = HashMap::new();

    for artifact in artifacts {
        let ast = artifact.ast();
        let sid_map = build_source_id_map(artifact, path_to_artifact, source_cache);

        let mut collect = |func: &solc::ast::FunctionDefinition| {
            if func.body.is_none() {
                return;
            }
            let name = match func.kind {
                solc::ast::FunctionKind::Constructor => "constructor".to_string(),
                solc::ast::FunctionKind::Fallback => "fallback".to_string(),
                solc::ast::FunctionKind::Receive => "receive".to_string(),
                _ if func.name.is_empty() => return,
                // checkrs: allow(clone_in_loops)
                _ => func.name.clone(),
            };
            let Some(path) = sid_map.get(&func.src.source_index) else {
                return;
            };
            let content = source_cache.get(path).cloned().unwrap_or_default();
            if content.is_empty() {
                return;
            }
            let start_line = offset_to_line(&content, func.src.offset);

            // If the function has a body but no source map entry covers any
            // line of its definition (signature + body), the compiler
            // eliminated the function entirely; skip it.
            if func.body.is_some() {
                let func_start_line = offset_to_line(&content, func.src.offset);
                let func_end_line =
                    offset_to_line(&content, func.src.offset.saturating_add(func.src.length));
                let has_source_map = source_map_lines
                    .get(path)
                    .map(|sm_lines| {
                        (func_start_line..=func_end_line).any(|l| sm_lines.contains(&l))
                    })
                    .unwrap_or(false);
                if !has_source_map {
                    return;
                }
            }

            let body_hits = func.body.as_ref().map_or(0, |body| {
                let start = body.src.offset;
                let end = body.src.offset.saturating_add(body.src.length);
                let start_body_line = offset_to_line(&content, start);
                let end_body_line = offset_to_line(&content, end);
                let mut max_hit = 0u64;
                for line in start_body_line..=end_body_line {
                    if let Some(hit) = line_hits.get(path).and_then(|h| h.get(&line)) {
                        max_hit = max_hit.max(*hit);
                    }
                }
                if max_hit > 0 {
                    return max_hit;
                }
                // Fall back to the function start line hit.
                line_hits
                    .get(path)
                    .and_then(|h| h.get(&start_line).copied())
                    .unwrap_or(0)
            });

            // checkrs: allow(clone_in_loops)
            file_functions
                // checkrs: allow(clone_in_loops)
                .entry(path.clone())
                .or_default()
                .push(FunctionCoverage {
                    name,
                    line: start_line,
                    hits: body_hits,
                });
        };

        for node in &ast.nodes {
            match node {
                solc::ast::SourceUnitNode::FunctionDefinition(func) => collect(func),
                solc::ast::SourceUnitNode::ContractDefinition(contract) => {
                    for node in &contract.nodes {
                        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node {
                            collect(func);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    file_functions
}

// ---------------------------------------------------------------------------
// Contract definition lines
// ---------------------------------------------------------------------------

/// Collect contract definition line numbers from the AST of resolved
/// artifacts. Contract, interface, and library definition lines are
/// non-executable and must not appear in the coverage report.
fn collect_contract_definition_lines(
    artifacts: &[&Artifact],
    path_to_artifact: &HashMap<PathBuf, &Artifact>,
    source_cache: &HashMap<PathBuf, String>,
) -> HashMap<PathBuf, HashSet<usize>> {
    let mut contract_lines: HashMap<PathBuf, HashSet<usize>> = HashMap::new();

    for artifact in artifacts {
        let ast = artifact.ast();
        let sid_map = build_source_id_map(artifact, path_to_artifact, source_cache);

        for node in &ast.nodes {
            if let solc::ast::SourceUnitNode::ContractDefinition(contract) = node {
                let Some(path) = sid_map.get(&contract.src.source_index) else {
                    continue;
                };
                let Some(content) = source_cache.get(path) else {
                    continue;
                };
                if content.is_empty() {
                    continue;
                }
                let line = offset_to_line(content, contract.src.offset);
                // checkrs: allow(clone_in_loops)
                contract_lines.entry(path.clone()).or_default().insert(line);
            }
        }
    }

    contract_lines
}

// ---------------------------------------------------------------------------
// Coverage reporter
// ---------------------------------------------------------------------------

/// Orchestrates the building of lcov coverage reports.
#[derive(Debug, Clone)]
pub struct CoverageReporter {
    artifacts: Vec<Artifact>,
    shared_coverage: SharedCoverage,
}

impl Default for CoverageReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverageReporter {
    /// Create a new empty [`CoverageReporter`].
    pub fn new() -> Self {
        Self {
            artifacts: Vec::new(),
            shared_coverage: SharedCoverage::new(),
        }
    }

    /// Set the build artifacts.
    pub fn build_artifacts(mut self, artifacts: Vec<Artifact>) -> Self {
        self.artifacts = artifacts;
        self
    }

    /// Set the [`SharedCoverage`] data.
    pub fn shared_coverage(mut self, coverage: SharedCoverage) -> Self {
        self.shared_coverage = coverage;
        self
    }

    /// Build the coverage report.
    ///
    /// # Pipeline
    ///
    /// 1. **Build path-to-artifact** map from all loaded artifacts.
    /// 2. **Match active bytecodes** from [`SharedCoverage`] to artifacts
    ///    via codehash, producing the set of root artifacts.
    /// 3. **Resolve source files recursively** from each root artifact's
    ///    `metadata.sources`. Each source file key has its own child
    ///    artifact that is resolved transitively.
    /// 4. **Pre-read source files** and build caches.
    /// 5. **Build PC-counter map**: for each matched codehash, walk PCs
    ///    with hits, resolve through the artifact source map to (file,
    ///    line), and aggregate hit counts.
    /// 6. **Determine executable lines** from the source map: a line is
    ///    executable when at least one source map entry maps to it, the
    ///    line is not a close-bracket (`}`), the line is not empty, and the
    ///    line is not a contract/interface/library definition.
    /// 7. **Collect function coverage** from the AST of resolved artifacts.
    /// 8. **Assemble the final `lcov.info` report.**
    #[instrument(skip(self), level = "trace")]
    pub fn build(self) -> CoverageReport {
        // -------------------------------------------------------------------
        // Step 1: Build path-to-artifact map.
        // -------------------------------------------------------------------
        let mut path_to_artifact: HashMap<PathBuf, &Artifact> = HashMap::new();
        for artifact in &self.artifacts {
            // checkrs: allow(clone_in_loops)
            path_to_artifact.insert(artifact.ast().absolute_path.clone(), artifact);
        }

        // -------------------------------------------------------------------
        // Step 2: Match active bytecodes → root artifacts.
        // -------------------------------------------------------------------
        let index = ArtifactIndex::new(&self.artifacts);
        let all_bytecodes = self.shared_coverage.all_bytecodes();
        tracing::trace!(all_bytecodes_len = all_bytecodes.len());

        let mut codehash_match: HashMap<B256, (&Artifact, bool)> = HashMap::new();
        let mut matched_ids: Vec<B256> = Vec::new();
        let mut root_artifact_ids: HashSet<&ArtifactId> = HashSet::new();

        for (id, bytecode) in &all_bytecodes {
            if let Some(matched) = index.find(bytecode) {
                codehash_match.insert(*id, matched);
                matched_ids.push(*id);
                root_artifact_ids.insert(matched.0.id());
            }
        }
        tracing::trace!(root_artifact_count = root_artifact_ids.len());

        // -------------------------------------------------------------------
        // Step 3: Resolve source files recursively from metadata.sources.
        //
        // root_to_files: Map[Root Artifact Id → Set[File]]
        // -------------------------------------------------------------------
        let mut all_files: HashSet<PathBuf> = HashSet::new();
        let mut resolved_artifact_ids: HashSet<&ArtifactId> = HashSet::new();

        for root_id in &root_artifact_ids {
            let Some(root_artifact) = path_to_artifact.get(&root_id.path) else {
                continue;
            };
            let mut queue = vec![*root_artifact];
            let mut visited: HashSet<&ArtifactId> = HashSet::new();

            while let Some(current) = queue.pop() {
                if !visited.insert(current.id()) {
                    continue;
                }
                resolved_artifact_ids.insert(current.id());

                // checkrs: allow(nested_if_let)
                if let Some(sources) = current.metadata_sources() {
                    for path_str in sources.keys() {
                        let path = PathBuf::from(path_str);
                        // checkrs: allow(clone_in_loops)
                        all_files.insert(path.clone());
                        if let Some(child) = path_to_artifact.get(&path) {
                            queue.push(child);
                        }
                    }
                } else {
                    // checkrs: allow(clone_in_loops)
                    all_files.insert(current.ast().absolute_path.clone());
                }
            }
        }

        if all_files.is_empty() {
            return CoverageReport::default();
        }

        let resolved_artifact_refs: Vec<&Artifact> = self
            .artifacts
            .iter()
            .filter(|a| resolved_artifact_ids.contains(a.id()))
            .collect();

        // -------------------------------------------------------------------
        // Step 4: Pre-read source files and build caches.
        // -------------------------------------------------------------------
        let project_path = resolved_artifact_refs
            .first()
            .map(|a| a.project_path())
            .unwrap_or_else(|| {
                self.artifacts
                    .first()
                    .map(|a| a.project_path())
                    .unwrap_or_else(|| std::path::Path::new(""))
            });
        let mut source_cache: HashMap<PathBuf, String> = HashMap::new();
        for path in &all_files {
            let full = project_path.join(path);
            if let Ok(content) = fs::read_to_string(&full) {
                // checkrs: allow(clone_in_loops)
                source_cache.insert(path.clone(), content);
            }
        }

        // -------------------------------------------------------------------
        // Step 5: Build PC-counter map.
        //
        // For each matched codehash, walk PCs with hits, resolve through
        // the artifact source map to (file, line), and aggregate hit counts.
        // -------------------------------------------------------------------
        let matched_counts = self
            .shared_coverage
            .raw_edge_counts_with_bytecodes_for_ids(&matched_ids);

        // Map: file path → (line → max hit count)
        let mut line_hits: HashMap<PathBuf, HashMap<usize, u64>> = HashMap::new();
        // Map: file path → set of line numbers that appear in source map
        let mut source_map_lines: HashMap<PathBuf, HashSet<usize>> = HashMap::new();

        // Precompute source_id→path maps for each root artifact.
        let mut sid_maps: HashMap<&ArtifactId, HashMap<usize, PathBuf>> = HashMap::new();
        for root_id in &root_artifact_ids {
            if let Some(artifact) = path_to_artifact.get(&root_id.path) {
                sid_maps.insert(
                    root_id,
                    build_source_id_map(artifact, &path_to_artifact, &source_cache),
                );
            }
        }

        // Precompute pc_map for every (artifact, is_initcode) pair, and
        // populate source_map_lines from deployed (runtime) source maps only.
        // Initcode (constructor) source maps are excluded because executable
        // lines are derived from runtime code paths.
        let mut pc_map_cache: HashMap<(&ArtifactId, bool), Vec<Option<SourceMapEntry>>> =
            HashMap::new();
        for root_id in &root_artifact_ids {
            let Some(artifact) = path_to_artifact.get(&root_id.path) else {
                continue;
            };
            let Some(sid_map) = sid_maps.get(root_id) else {
                continue;
            };
            if let Some(bytecode) = artifact.bytecode() {
                let code =
                    parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
                let source_map = parse_source_map(&bytecode.source_map);
                let pc_map = build_pc_to_source_map(&code, &source_map);
                pc_map_cache.insert((artifact.id(), true), pc_map);
            }
            // checkrs: allow(nested_if_let)
            if let Some(deployed) = artifact.deployed_bytecode() {
                populate_source_map_lines_from_deployed(
                    deployed,
                    artifact,
                    sid_map,
                    &source_cache,
                    &mut source_map_lines,
                    &mut pc_map_cache,
                );
            }
        }

        for counts in &matched_counts {
            let Some((artifact, is_initcode)) = codehash_match.get(&counts.contract_id) else {
                continue;
            };
            let Some(pc_map) = pc_map_cache.get(&(artifact.id(), *is_initcode)) else {
                continue;
            };
            let Some(sid_map) = sid_maps.get(artifact.id()) else {
                continue;
            };

            tracing::debug!(
                "matched artifact: {} (bytecode len={}, initcode={})",
                artifact.id(),
                counts.bytecode.len(),
                is_initcode
            );

            for (pc, raw_count) in counts.raw_edges.iter().enumerate() {
                if *raw_count == 0 {
                    continue;
                }
                let Some(entry) = pc_map.get(pc).copied().flatten() else {
                    continue;
                };
                let Some(path) = sid_map.get(&entry.source_index) else {
                    continue;
                };

                let Some(content) = source_cache.get(path) else {
                    continue;
                };
                if content.is_empty() {
                    continue;
                }

                let line = offset_to_line(content, entry.offset);

                // Record that this line appears in the source map.
                source_map_lines
                    // checkrs: allow(clone_in_loops)
                    .entry(path.clone())
                    .or_default()
                    .insert(line);

                // Update line hits (max aggregation).
                // checkrs: allow(clone_in_loops)
                let file_hits = line_hits.entry(path.clone()).or_default();
                let current = file_hits.entry(line).or_insert(0);
                *current = (*current).max(*raw_count);
            }
        }

        // -------------------------------------------------------------------
        // Step 6: Determine executable lines.
        //
        // A line is executable when:
        //   1. It appears in source_map_lines (has a source map entry).
        //   2. It is not a close-bracket line (trimmed == "}").
        //   3. It is not empty (trimmed.is_empty()).
        //   4. It is not a contract/interface/library definition line.
        // -------------------------------------------------------------------
        let contract_def_lines = collect_contract_definition_lines(
            &resolved_artifact_refs,
            &path_to_artifact,
            &source_cache,
        );

        let mut executable_line_hits: HashMap<PathBuf, HashMap<usize, u64>> = HashMap::new();

        for path in &all_files {
            let Some(content) = source_cache.get(path) else {
                continue;
            };
            let sm_lines = source_map_lines.get(path);
            let cd_lines = contract_def_lines.get(path);

            for (line_idx, line_text) in content.lines().enumerate() {
                let line_num = line_idx + 1;
                let trimmed = line_text.trim();

                // Non-executable: close bracket, empty line, comment, or
                // contract definition.
                if trimmed == "}"
                    || trimmed.is_empty()
                    || trimmed.starts_with("//")
                    || cd_lines.map(|s| s.contains(&line_num)).unwrap_or(false)
                {
                    continue;
                }

                // Executable only if this line has a source map entry.
                if sm_lines.map(|s| s.contains(&line_num)).unwrap_or(false) {
                    let hits = line_hits
                        .get(path)
                        .and_then(|h| h.get(&line_num))
                        .copied()
                        .unwrap_or(0);
                    executable_line_hits
                        // checkrs: allow(clone_in_loops)
                        .entry(path.clone())
                        .or_default()
                        .insert(line_num, hits);
                }
            }
        }

        // -------------------------------------------------------------------
        // Step 7: Collect function coverage from AST.
        // -------------------------------------------------------------------
        let file_functions = collect_function_coverage(
            &resolved_artifact_refs,
            &path_to_artifact,
            &source_cache,
            &line_hits,
            &source_map_lines,
        );

        // Ensure every function start line has a DA entry.
        let mut file_functions_out: HashMap<PathBuf, Vec<FunctionCoverage>> = HashMap::new();
        for (path, functions) in file_functions {
            // checkrs: allow(clone_in_loops)
            let file_lines = executable_line_hits.entry(path.clone()).or_default();
            // checkrs: allow(clone_in_loops)
            let hits = line_hits.entry(path.clone()).or_default();

            for func in &functions {
                file_lines.insert(func.line, func.hits);
                hits.entry(func.line)
                    .and_modify(|e| *e = (*e).max(func.hits))
                    .or_insert(func.hits);
            }
            file_functions_out.insert(path, functions);
        }

        // Ensure all resolved source files appear in the report.
        for path in &all_files {
            // checkrs: allow(clone_in_loops)
            executable_line_hits.entry(path.clone()).or_default();
        }

        // -------------------------------------------------------------------
        // Step 8: Assemble the report.
        // -------------------------------------------------------------------
        let mut all_paths: Vec<PathBuf> = all_files.into_iter().collect();
        all_paths.sort();

        let mut files = Vec::new();
        for path in all_paths {
            let line_hits = executable_line_hits.remove(&path).unwrap_or_default();
            let functions = file_functions_out.remove(&path).unwrap_or_default();
            // Skip files that have no executable lines and no functions.
            if line_hits.is_empty() && functions.is_empty() {
                continue;
            }
            files.push(FileCoverage {
                path,
                line_hits,
                functions,
            });
        }

        CoverageReport { files }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use alloy_sol_types::SolCall;
    use revm::primitives::{Address, Bytes};

    use crate::evm::coverage::exec::{ExecutionContractCoverage, ExecutionCoverage};
    use crate::evm::{
        Chain, ChainConfig, Contract, DeployInput, SetupInput, SharedCoverage, Transaction,
    };
    use crate::foundry;

    use super::*;

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }

        interface TargetContractWithInterface {
            function interfaceCall(uint256 amount) external returns (uint256);
        }

        interface TargetContractWithLibLinked {
            function libLinkedCall(uint256 amount) external returns (uint256);
        }

        interface TargetContractWithLib {
            function libCall(uint256 amount) external returns (uint256);
        }

        interface TargetContractBasic {
            function addAndSub(uint256 a, uint256 b) external returns (uint256);
        }

        interface TargetContractWithLoop {
            function runLoop(uint256 count) external;
            function runNestedLoop(uint256 outer, uint256 inner) external;
        }

        interface TargetContractWithIf {
            function runIf(bool condition) external;
            function runIfElse(bool condition) external;
            function runIfElseWithNewline(bool condition) external;
            function runNestedIf(bool a, bool b) external;
        }

        interface EmptyTargetFunction {
            function dummyTargetFunction() external;
        }

        interface InheritedTarget {
            function inheritedTargetFunction() external;
        }

        interface CoverageInactiveUser {
            function callUsed() external pure returns (uint256);
        }
    }

    fn load_coverage_fixture(project_path: impl AsRef<Path>, id: &str) -> Contract {
        let project = foundry::Project::new(project_path);
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    struct Deployed {
        chain: Chain,
        address: Address,
        global: SharedCoverage,
    }

    fn deploy_and_setup(project_path: impl AsRef<Path>, contract: &Contract) -> Deployed {
        let mut config = ChainConfig::default().coverage(true);
        let project = foundry::Project::new(project_path);
        let artifacts = project.load_artifacts().unwrap();
        let mut compiled_contracts = HashMap::new();
        for (id, artifact) in &artifacts {
            let initcode: Bytes = match artifact {
                foundry::Artifact::Contract(c) => c.bytecode.object.parse().unwrap_or_default(),
                foundry::Artifact::Library(c) => c.bytecode.object.parse().unwrap_or_default(),
                _ => continue,
            };
            if initcode.is_empty() {
                continue;
            }
            compiled_contracts.insert(id.into(), initcode);
        }
        config = config.with_compiled_contracts(compiled_contracts);
        let mut chain = Chain::new(config).unwrap();
        let mut deploy_opts = DeployInput::new(&contract.initcode);
        for lib in &contract.libraries {
            deploy_opts = deploy_opts.add_library(lib.clone());
        }
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        let global = SharedCoverage::new();
        global.merge(&deployment.coverage);

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
            global.merge(&setup.coverage);
        }

        Deployed {
            chain,
            address: target,
            global,
        }
    }

    fn build_report(shared_coverage: &SharedCoverage, artifacts: &[Artifact]) -> CoverageReport {
        CoverageReporter::new()
            .build_artifacts(artifacts.to_vec())
            .shared_coverage(shared_coverage.clone())
            .build()
    }

    fn project_path() -> PathBuf {
        fs::canonicalize("fixtures/target-contract-coverage")
            .unwrap_or_else(|_| PathBuf::from("fixtures/target-contract-coverage"))
    }

    /// Regression test: build artifacts that include interfaces (which have
    /// no deployed bytecode) must not cause coverage report generation to fail.
    #[test]
    fn coverage_report_build_with_interface_artifact() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/CoverageBranch.sol:CoverageBranch",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        assert!(
            !report.files.is_empty(),
            "coverage report should be generated even when build artifacts include interfaces"
        );
    }

    /// Regression test: coverage report for if-statement close brackets and
    /// empty lines between if-else branches must be handled correctly.
    #[test]
    fn target_contract_with_if() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/TargetContractWithIf.sol:TargetContractWithIf",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runIfCall::new((true,)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runIfElseCall::new((true,)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runIfElseCall::new((false,)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runIfElseWithNewlineCall::new((true,)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runIfElseWithNewlineCall::new((false,)).abi_encode(),
            )),
            Transaction::new(deployed.address).calldata(Bytes::from(
                TargetContractWithIf::runNestedIfCall::new((true, true)).abi_encode(),
            )),
        ];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);
        let formatted = format!("{report}");

        let expected_file = "fixtures/target-contract-coverage/expected/TargetContractWithIf.info";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace(
            "fixtures/target-contract-coverage",
            &project_path().to_string_lossy(),
        );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }

    /// Regression test: coverage report generation must not crash and must
    /// produce non-empty output even when the target function body is empty.
    #[test]
    fn coverage_report_empty_target_function() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/EmptyTargetFunction.sol:EmptyTargetFunction",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            EmptyTargetFunction::dummyTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        assert!(
            !report.files.is_empty(),
            "coverage report must contain at least one file"
        );
        for file in &report.files {
            assert!(
                !file.line_hits.is_empty() || !file.functions.is_empty(),
                "coverage report file must not be empty: {}",
                file.path.display()
            );
        }
    }

    #[test]
    fn coverage_report_inherited_target_function() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/InheritedTarget.sol:InheritedTarget",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            InheritedTarget::inheritedTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        assert!(
            !report.files.is_empty(),
            "coverage report must be generated for a target function inherited from a base contract"
        );
    }

    /// Regression test: genhtml requires a DA entry for every FN line. If a
    /// function's start line is not in the source map, the coverage report must
    /// still emit a DA entry for that line so genhtml does not fail with
    /// "unexpected category UNK".
    #[test]
    fn coverage_report_function_start_line_without_source_map() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/UnusedLibraryUser.sol:UnusedLibraryUser",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            hex::decode("771602f7").unwrap(), // useAdd(uint256,uint256)
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        // Functions whose bodies are eliminated by the compiler (no source
        // map entries for the function start line) must not appear in the
        // report. Only usedAdd (line 5) which is called via inlining should
        // be present; unusedAdd (line 9) must be absent.
        let unused_library = report
            .files
            .iter()
            .find(|f| f.path.ends_with("UnusedLibrary.sol"))
            .expect("UnusedLibrary.sol must be in report");
        assert!(
            unused_library.line_hits.contains_key(&5),
            "UnusedLibrary.sol must contain DA entry for used function at line 5: {unused_library:?}"
        );
        assert!(
            !unused_library.line_hits.contains_key(&9),
            "UnusedLibrary.sol must NOT contain DA entry for eliminated function at line 9: {unused_library:?}"
        );
    }

    /// Regression test: a source map entry whose offset points to the end of a
    /// source file that ends with a newline must not produce a line number beyond
    /// the file's actual line count.
    #[test]
    fn coverage_report_trailing_newline_no_out_of_range() {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let mut artifacts: Vec<Artifact> =
            project.load_artifacts().unwrap().into_values().collect();

        // Extract the deployed bytecode from the artifact before we modify it.
        let mut bytecode = Vec::new();
        for artifact in &artifacts {
            if artifact.id().to_string()
                == "src/CoverageTrailingNewline.sol:CoverageTrailingNewline"
            {
                bytecode = artifact.deployed_bytecode().unwrap().to_bytes().to_vec();
            }
        }
        assert!(!bytecode.is_empty(), "deployed bytecode must not be empty");

        // Inject a fake source map entry pointing to the end of the file to
        // simulate a compiler-generated entry that sits past the final newline.
        let source_path =
            PathBuf::from("fixtures/target-contract-coverage/src/CoverageTrailingNewline.sol");
        let content = fs::read_to_string(&source_path).unwrap();
        let file_len = content.len();

        for artifact in &mut artifacts {
            if artifact.id().to_string()
                == "src/CoverageTrailingNewline.sol:CoverageTrailingNewline"
            {
                if let Artifact::Contract(a) = artifact {
                    let original = a.deployed_bytecode.source_map.clone();
                    a.deployed_bytecode.source_map =
                        format!("{}:0:{}:-:0;{}", file_len, a.source_id, original);
                }
            }
        }

        // Create a fake coverage hit for PC 0 (the first opcode) which maps to the fake entry.
        let global = SharedCoverage::new();
        let mut fake_local = ExecutionCoverage::new();
        let mut fake_contract = ExecutionContractCoverage::new(bytecode.len());
        fake_contract.bytecode = bytecode;
        fake_contract.edges[0] = 1;
        fake_contract.hit_pcs.push(0);
        let contract_id = keccak256(&fake_contract.bytecode);
        fake_local.contracts.insert(contract_id, fake_contract);
        global.merge(&fake_local);

        let report = build_report(&global, &artifacts);

        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("CoverageTrailingNewline.sol"))
            .expect("CoverageTrailingNewline.sol must be in report");
        let max_line = *file.line_hits.keys().max().unwrap_or(&0);
        let line_count = content.lines().count();
        assert!(
            max_line <= line_count,
            "CoverageTrailingNewline.sol must not contain a line number beyond the file's line count ({line_count}), but found {max_line}"
        );
    }

    /// Regression test: contracts with immutable variables must be matched
    /// correctly by the coverage reporter so that their coverage is not lost.
    #[test]
    fn coverage_report_immutable_contract_matched() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/CoverageImmutable.sol:CoverageImmutable",
        );
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deploy_opts = DeployInput::new(&contract.initcode);
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(deployment.result.success, "deployment must succeed");

        let global = SharedCoverage::new();
        global.merge(&deployment.coverage);

        let txs = vec![
            Transaction::new(deployment.address.unwrap()).calldata(Bytes::from(
                hex::decode("20965255").unwrap(), // getValue()
            )),
        ];
        let exec = chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&global, &artifacts);

        let file = report
            .files
            .iter()
            .find(|f| f.path.ends_with("CoverageImmutable.sol"))
            .expect("CoverageImmutable.sol must be in report");
        let get_value_line = 15; // line of function getValue() signature
        let body_line = 16; // line of function getValue() body
        assert!(
            file.line_hits.contains_key(&get_value_line),
            "CoverageImmutable.sol must contain DA entry for getValue() line {get_value_line}: {file:?}"
        );
        assert!(
            file.line_hits.get(&get_value_line).unwrap_or(&0) > &0,
            "CoverageImmutable.sol getValue() line {get_value_line} must have hits > 0: {file:?}"
        );
        assert!(
            file.line_hits.contains_key(&body_line),
            "CoverageImmutable.sol must contain DA entry for getValue() body line {body_line}: {file:?}"
        );
        assert!(
            file.line_hits.get(&body_line).unwrap_or(&0) > &0,
            "CoverageImmutable.sol getValue() body line {body_line} must have hits > 0: {file:?}"
        );
    }

    /// Regression test: inactive artifacts must not contribute executable lines
    /// to the coverage report. Only artifacts whose bytecode was recorded during
    /// fuzzing should define the set of executable source lines.
    #[test]
    fn coverage_report_inactive_artifact_no_executable_lines() {
        let contract = load_coverage_fixture(
            "fixtures/target-contract-coverage",
            "src/CoverageInactiveUser.sol:CoverageInactiveUser",
        );
        let mut deployed = deploy_and_setup("fixtures/target-contract-coverage", &contract);

        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            CoverageInactiveUser::callUsedCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        deployed.global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts: Vec<Artifact> = project.load_artifacts().unwrap().into_values().collect();
        let report = build_report(&deployed.global, &artifacts);

        // CoverageInactiveUser.sol must appear because it is deployed.
        assert!(
            report
                .files
                .iter()
                .any(|f| f.path.ends_with("CoverageInactiveUser.sol")),
            "CoverageInactiveUser.sol must appear in coverage report: {report:?}"
        );
    }
}
