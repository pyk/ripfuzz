//! Coverage reporter that generates an lcov.info file from solc output and
//! shared coverage data.
//!
//! [`CoverageReporter`] is the entry point. It takes two inputs:
//!
//! 1. **Solc output** - a compiled [`SolcOutput`] that contains bytecode,
//!    source maps, and ASTs for the compilation unit.
//! 2. [`SharedCoverage`] - raw per-PC hit counts collected during fuzzing.
//!
//! The reporter resolves every hit PC back to a source line using the solc
//! source maps, then aggregates the results into a single `lcov.info` report.
//!
//! # Two-tier coverage model
//!
//! The reporter uses a two-tier strategy to produce accurate coverage data
//! even when the Solidity optimizer deduplicates code paths (sharing return
//! sequences across internal functions):
//!
//! ## Tier 1: per-function hit counts (FNDA)
//!
//! Function hit counts are derived from the raw count of each function's
//! *entry JUMPDEST*: the first bytecode instruction whose source-map entry
//! maps to the function's AST source range. Because the optimizer never
//! merges distinct function entry points (callers must jump to unique
//! addresses), the entry-PC count is always an accurate call count, free
//! from the inflation that affects shared epilogue / clean-up blocks.
//!
//! ## Tier 2: binary line hits (DA)
//!
//! Per-line hit counts are reported as **binary** (1 = hit, 0 = not hit).
//! When the optimizer merges post-call return paths, multiple functions may
//! execute the same bytecode. The source map attributes that code to the
//! first function that contained it, causing raw hit counts on those PCs
//! to be higher than any single function's call count. By collapsing every
//! line to a binary value the reporter avoids misleading counts while still
//! faithfully indicating whether a line was exercised.
//!
//! ## lcov.info mapping
//!
//! | lcov field | Source |
//! |---|---|
//! | `FNDA:<count>,<name>` | Raw count of the function's entry-PC |
//! | `DA:<line>,<hits>` | 1 if the line is executable *and* at least one PC maps to it, else 0. Function-definition lines carry the same count as their FNDA entry. |
//!
//! # Pipeline
//!
//! 1. **Build a compiled-contract index** from `output.contracts` plus AST
//!    library kinds.
//! 2. **Match active bytecodes** from [`SharedCoverage`] to compiled
//!    contracts via codehash (masking out link references and immutables).
//! 3. **Build one compilation-unit source-id map** from `output.sources`.
//! 4. **Pre-read source files** and build caches.
//! 5. **Build PC-counter map**: for each matched codehash, walk PCs with
//!    hits, resolve through the contract source map to (file, line), and
//!    record *binary* line hit markers.
//! 6. **Determine executable lines** from the source map: a line is
//!    executable when at least one source map entry maps to it, the line
//!    is not a close-bracket (`}`), the line is not empty, and the line is
//!    not a contract/interface/library definition.
//! 7. **Collect function coverage** from the AST of compilation sources,
//!    using entry-PC raw counts for accurate per-function hit counts.
//! 8. **Assemble the final `lcov.info` report.**
//!
//! # Usage
//!
//! ```text
//! let report = CoverageReporter::new()
//!     .solc_output(&solc_output)
//!     .shared_coverage(shared_coverage)
//!     .base_project_path(root)
//!     .build();
//! let lcov_info = format!("{report}");
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_primitives::{B256, keccak256};
use solc::{Bytecode, LinkReference, StandardJSONOutput};
use tracing::{debug, instrument, trace};

use crate::compilers::solc::SolcOutput;
use crate::evm::coverage::id::CoverageId;
use crate::evm::coverage::shared::{RawEdgeCounts, SharedCoverage};
use crate::evm::coverage::source_map::{SourceMapEntry, parse_source_map};

type LinkReferences = HashMap<String, HashMap<String, Vec<LinkReference>>>;

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

fn bytecode_object(bytecode: &Bytecode) -> &str {
    bytecode.object.as_deref().unwrap_or("")
}

fn bytecode_source_map(bytecode: &Bytecode) -> &str {
    bytecode.source_map.as_deref().unwrap_or("")
}

fn bytecode_link_refs(bytecode: &Bytecode) -> LinkReferences {
    bytecode.link_references.clone().unwrap_or_default()
}

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

struct CompiledContract<'a> {
    path: PathBuf,
    name: &'a str,
    bytecode: Option<&'a Bytecode>,
    deployed: Option<&'a Bytecode>,
    is_library: bool,
}

fn is_library(ast: &solc::ast::SourceUnit, name: &str) -> bool {
    ast.nodes.iter().any(|node| {
        if let solc::ast::SourceUnitNode::ContractDefinition(contract) = node {
            contract.name == name && contract.contract_kind == solc::ast::ContractKind::Library
        } else {
            false
        }
    })
}

fn compiled_contracts(output: &StandardJSONOutput) -> Vec<CompiledContract<'_>> {
    let mut contracts = Vec::new();
    for (path, named) in &output.contracts {
        let ast = output
            .sources
            .get(path)
            .and_then(|source| source.ast.as_ref());
        for (name, contract) in named {
            let Some(evm) = contract.evm.as_ref() else {
                continue;
            };
            let is_library = match ast {
                Some(ast) => is_library(ast, name),
                None => false,
            };
            contracts.push(CompiledContract {
                // checkrs: allow(clone_in_loops)
                path: path.clone(),
                name,
                bytecode: evm.bytecode.as_ref(),
                deployed: evm.deployed_bytecode.as_ref(),
                is_library,
            });
        }
    }
    contracts
}

fn deployed_index_entry(
    contract_idx: usize,
    contract: &CompiledContract<'_>,
) -> Option<BytecodeIndexEntry> {
    let deployed = contract.deployed?;
    let link_refs = bytecode_link_refs(deployed);
    let code = parse_bytecode_with_placeholders(bytecode_object(deployed), &link_refs);
    if code.is_empty() {
        return None;
    }
    let mut positions = collect_link_positions(&link_refs);
    if let Some(immutables) = deployed.immutable_references.as_ref() {
        for refs in immutables.values() {
            for r in refs {
                positions.push((r.start, r.length));
            }
        }
    }
    if contract.is_library && code.first() == Some(&0x73) {
        positions.push((1, 20));
    }
    let code_len = code.len();
    let mut masked = code;
    zero_out_positions(&mut masked, &positions);
    Some(BytecodeIndexEntry {
        contract_idx,
        hash: keccak256(&masked),
        positions,
        is_initcode: false,
        code_len,
    })
}

fn initcode_index_entry(
    contract_idx: usize,
    contract: &CompiledContract<'_>,
) -> Option<BytecodeIndexEntry> {
    let bytecode = contract.bytecode?;
    let link_refs = bytecode_link_refs(bytecode);
    let code = parse_bytecode_with_placeholders(bytecode_object(bytecode), &link_refs);
    if code.is_empty() {
        return None;
    }
    let positions = collect_link_positions(&link_refs);
    let code_len = code.len();
    let mut masked = code;
    zero_out_positions(&mut masked, &positions);
    Some(BytecodeIndexEntry {
        contract_idx,
        hash: keccak256(&masked),
        positions,
        is_initcode: true,
        code_len,
    })
}

struct BytecodeIndexEntry {
    contract_idx: usize,
    hash: B256,
    positions: Vec<(usize, usize)>,
    is_initcode: bool,
    code_len: usize,
}

struct BytecodeIndex {
    entries: Vec<BytecodeIndexEntry>,
    entries_by_len: HashMap<usize, Vec<usize>>,
}

impl BytecodeIndex {
    fn new(contracts: &[CompiledContract<'_>]) -> Self {
        let mut entries = Vec::new();
        for (contract_idx, contract) in contracts.iter().enumerate() {
            if let Some(entry) = deployed_index_entry(contract_idx, contract) {
                entries.push(entry);
            }
            if let Some(entry) = initcode_index_entry(contract_idx, contract) {
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

    fn find(&self, raw_bytecode: &[u8]) -> Option<(usize, bool)> {
        let candidates = self.entries_by_len.get(&raw_bytecode.len())?;
        let mut masked = raw_bytecode.to_vec();
        for idx in candidates {
            let entry = &self.entries[*idx];
            zero_out_positions(&mut masked, &entry.positions);
            if keccak256(&masked) == entry.hash {
                return Some((entry.contract_idx, entry.is_initcode));
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

/// Walk a deployed bytecode's source map and populate `source_map_lines`
/// with every line that has a source map entry. Also insert the pc_map into
/// `pc_map_cache`.
fn populate_source_map_lines_from_deployed(
    deployed: &Bytecode,
    contract_idx: usize,
    sid_map: &HashMap<usize, PathBuf>,
    source_cache: &HashMap<PathBuf, String>,
    source_map_lines: &mut HashMap<PathBuf, HashSet<usize>>,
    pc_map_cache: &mut HashMap<(usize, bool), Vec<Option<SourceMapEntry>>>,
) {
    let link_refs = bytecode_link_refs(deployed);
    let code = parse_bytecode_with_placeholders(bytecode_object(deployed), &link_refs);
    let source_map = parse_source_map(bytecode_source_map(deployed));
    let pc_map = build_pc_to_source_map(&code, &source_map);
    for entry in source_map.iter() {
        if entry.source_index < 0 {
            continue;
        }
        let idx = entry.source_index as usize;
        if let Some(path) = sid_map.get(&idx)
            && let Some(content) = source_cache.get(path)
        {
            let line = offset_to_line(content, entry.offset);
            // checkrs: allow(clone_in_loops)
            let path_key = path.clone();
            source_map_lines.entry(path_key).or_default().insert(line);
        }
    }
    pc_map_cache.insert((contract_idx, false), pc_map);
}

/// Collect function definitions from compilation-unit ASTs by walking each
/// source file once.
///
/// For each function the hit count is taken from the raw coverage count of
/// the function's *entry PC*: the earliest PC whose source-map entry
/// falls within the function's AST source range. Entry PCs are never shared
/// by the optimizer, so this count accurately reflects how many times the
/// function was entered during the campaign.
fn collect_function_coverage(
    output: &StandardJSONOutput,
    sid_map: &HashMap<usize, PathBuf>,
    source_cache: &HashMap<PathBuf, String>,
    source_map_lines: &HashMap<PathBuf, HashSet<usize>>,
    matched_counts: &[RawEdgeCounts],
    pc_map_cache: &HashMap<(usize, bool), Vec<Option<SourceMapEntry>>>,
    codehash_match: &HashMap<CoverageId, (usize, bool)>,
) -> HashMap<PathBuf, Vec<FunctionCoverage>> {
    let mut file_functions: HashMap<PathBuf, Vec<FunctionCoverage>> = HashMap::new();

    let mut collect = |func: &solc::ast::FunctionDefinition| {
        if func.body.is_none() {
            return;
        }
        let name = match func.kind {
            Some(solc::ast::FunctionKind::Constructor) => "constructor".to_string(),
            Some(solc::ast::FunctionKind::Fallback) => "fallback".to_string(),
            Some(solc::ast::FunctionKind::Receive) => "receive".to_string(),
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
        // eliminated the function entirely. Skip it.
        let func_start_line = offset_to_line(&content, func.src.offset);
        let func_end_line =
            offset_to_line(&content, func.src.offset.saturating_add(func.src.length));
        let has_source_map = source_map_lines
            .get(path)
            .map(|sm_lines| (func_start_line..=func_end_line).any(|l| sm_lines.contains(&l)))
            .unwrap_or(false);
        if !has_source_map {
            return;
        }

        // Tier 1: look up the function entry-PC raw count.
        // Walk every matched bytecode, scan the pc_map for the earliest
        // PC within the function's source range (correct source file +
        // offset), and use its raw hit count.
        //
        // A function may be defined in an abstract contract whose bytecode
        // lives in a derived concrete contract, so we resolve through the
        // compilation-unit sid_map rather than requiring the contract
        // names to be equal.
        //
        // Constructors only exist in initcode. Runtime functions only
        // exist in deployed bytecode. Filter by is_initcode so that
        // a deployed-bytecode source-map entry that happens to fall
        // within the constructor's source range cannot shadow the
        // real initcode entry.
        let func_src_start = func.src.offset;
        let func_src_end = func.src.offset.saturating_add(func.src.length);
        let target_initcode = matches!(func.kind, Some(solc::ast::FunctionKind::Constructor));
        let mut entry_hits: u64 = 0;

        for counts in matched_counts {
            let Some((contract_idx, is_initcode)) = codehash_match.get(&counts.contract_id) else {
                continue;
            };
            // Only match constructors against initcode, runtime
            // functions against deployed bytecode.
            if *is_initcode != target_initcode {
                continue;
            }
            let Some(pc_map) = pc_map_cache.get(&(*contract_idx, *is_initcode)) else {
                continue;
            };

            // Find the earliest PC whose source-map entry falls
            // within the function's AST source range *and* whose
            // resolved source file matches the function's source
            // file. For runtime functions this will be a JUMPDEST.
            // For initcode (constructors) it will be the first
            // instruction.
            let mut best_pc: Option<usize> = None;
            for (pc, entry) in pc_map.iter().enumerate() {
                let Some(e) = entry else {
                    continue;
                };
                if e.source_index < 0 {
                    continue;
                }
                // Only match entries from the same source file.
                let Some(entry_path) = sid_map.get(&(e.source_index as usize)) else {
                    continue;
                };
                if entry_path != path {
                    continue;
                }
                if e.offset < func_src_start || e.offset >= func_src_end {
                    continue;
                }
                best_pc = Some(match best_pc {
                    None => pc,
                    Some(prev) => prev.min(pc),
                });
            }

            entry_hits = best_pc
                .and_then(|pc| counts.raw_edges.get(pc).copied())
                .map_or(entry_hits, |hits| entry_hits.max(hits));
        }

        // checkrs: allow(clone_in_loops)
        file_functions
            // checkrs: allow(clone_in_loops)
            .entry(path.clone())
            .or_default()
            .push(FunctionCoverage {
                name,
                line: start_line,
                hits: entry_hits,
            });
    };

    for source in output.sources.values() {
        let Some(ast) = source.ast.as_ref() else {
            continue;
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

/// Collect contract definition line numbers from the AST of compilation
/// sources. Contract, interface, and library definition lines are
/// non-executable and must not appear in the coverage report.
fn collect_contract_definition_lines(
    output: &StandardJSONOutput,
    sid_map: &HashMap<usize, PathBuf>,
    source_cache: &HashMap<PathBuf, String>,
) -> HashMap<PathBuf, HashSet<usize>> {
    let mut contract_lines: HashMap<PathBuf, HashSet<usize>> = HashMap::new();

    for source in output.sources.values() {
        let Some(ast) = source.ast.as_ref() else {
            continue;
        };
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

/// Orchestrates the building of lcov coverage reports.
#[derive(Debug, Clone)]
pub struct CoverageReporter {
    output: Option<StandardJSONOutput>,
    shared_coverage: SharedCoverage,
    /// The project path to use as the base for resolving source files.
    /// Source keys in the solc output are relative to this directory.
    base_project_path: Option<PathBuf>,
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
            output: None,
            shared_coverage: SharedCoverage::new(),
            base_project_path: None,
        }
    }

    /// Set the solc compilation output used to resolve bytecode and sources.
    pub fn solc_output(mut self, output: &SolcOutput) -> Self {
        self.output = Some(output.output.clone());
        self
    }

    /// Set the [`SharedCoverage`] data.
    pub fn shared_coverage(mut self, coverage: SharedCoverage) -> Self {
        self.shared_coverage = coverage;
        self
    }

    /// Set the base project path for source file resolution.
    ///
    /// Source paths in the solc output are resolved relative to this
    /// directory so that lcov `SF:` entries match the compilation keys.
    pub fn base_project_path(mut self, path: impl AsRef<Path>) -> Self {
        let p = path.as_ref();
        self.base_project_path = Some(p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        self
    }

    /// Build the coverage report.
    ///
    /// # Pipeline
    ///
    /// 1. **Build a compiled-contract index** from `output.contracts`.
    /// 2. **Match active bytecodes** from [`SharedCoverage`] to compiled
    ///    contracts via codehash.
    /// 3. **Build one compilation-unit source-id map** from `output.sources`.
    /// 4. **Pre-read source files** and build caches.
    /// 5. **Build PC-counter map**: for each matched codehash, walk PCs
    ///    with hits, resolve through the contract source map to (file,
    ///    line), and aggregate hit counts.
    /// 6. **Determine executable lines** from the source map: a line is
    ///    executable when at least one source map entry maps to it, the
    ///    line is not a close-bracket (`}`), the line is not empty, and the
    ///    line is not a contract/interface/library definition.
    /// 7. **Collect function coverage** from the AST of compilation sources.
    /// 8. **Assemble the final `lcov.info` report.**
    #[instrument(skip(self), level = "trace")]
    pub fn build(self) -> CoverageReport {
        let Some(output) = self.output else {
            return CoverageReport::default();
        };

        // 1. Build a compiled-contract index from the solc output.
        let contracts = compiled_contracts(&output);
        let index = BytecodeIndex::new(&contracts);

        // 2. Match active bytecodes to compiled contracts.
        let all_bytecodes = self.shared_coverage.all_bytecodes();
        trace!(all_bytecodes_len = all_bytecodes.len());

        let mut codehash_match: HashMap<CoverageId, (usize, bool)> = HashMap::new();
        let mut matched_ids: Vec<CoverageId> = Vec::new();
        let mut matched_indices: HashSet<usize> = HashSet::new();

        for (id, bytecode) in &all_bytecodes {
            if let Some(matched) = index.find(bytecode) {
                codehash_match.insert(*id, matched);
                matched_ids.push(*id);
                matched_indices.insert(matched.0);
            }
        }
        trace!(matched_contract_count = matched_indices.len());

        if matched_indices.is_empty() {
            return CoverageReport::default();
        }

        // 3. Build one compilation-unit source-id map from output.sources.
        let mut sid_map: HashMap<usize, PathBuf> = HashMap::new();
        let mut all_files: HashSet<PathBuf> = HashSet::new();
        for (path, source) in &output.sources {
            if source.id >= 0 {
                // checkrs: allow(clone_in_loops)
                sid_map.insert(source.id as usize, path.clone());
            }
            // checkrs: allow(clone_in_loops)
            all_files.insert(path.clone());
        }

        if all_files.is_empty() {
            return CoverageReport::default();
        }

        // 4. Pre-read source files and build caches.
        let base = self.base_project_path.as_deref();
        let mut source_cache: HashMap<PathBuf, String> = HashMap::new();
        for path in &all_files {
            let full = match base {
                Some(base) => base.join(path),
                // checkrs: allow(clone_in_loops)
                None => path.clone(),
            };
            if let Ok(content) = fs::read_to_string(&full) {
                // checkrs: allow(clone_in_loops)
                source_cache.insert(path.clone(), content);
            } else if let Ok(content) = fs::read_to_string(path) {
                // checkrs: allow(clone_in_loops)
                source_cache.insert(path.clone(), content);
            }
        }

        // 5. Build PC-counter map.
        //
        // For each matched codehash, walk PCs with hits, resolve through
        // the contract source map to (file, line), and aggregate hit counts.
        let matched_counts = self
            .shared_coverage
            .raw_edge_counts_with_bytecodes_for_ids(&matched_ids);

        let mut line_hits: HashMap<PathBuf, HashMap<usize, u64>> = HashMap::new();
        let mut source_map_lines: HashMap<PathBuf, HashSet<usize>> = HashMap::new();
        let mut pc_map_cache: HashMap<(usize, bool), Vec<Option<SourceMapEntry>>> = HashMap::new();

        for contract_idx in &matched_indices {
            let contract = &contracts[*contract_idx];
            if let Some(bytecode) = contract.bytecode {
                let link_refs = bytecode_link_refs(bytecode);
                let code = parse_bytecode_with_placeholders(bytecode_object(bytecode), &link_refs);
                let source_map = parse_source_map(bytecode_source_map(bytecode));
                let pc_map = build_pc_to_source_map(&code, &source_map);
                pc_map_cache.insert((*contract_idx, true), pc_map);
            }
            if let Some(deployed) = contract.deployed {
                populate_source_map_lines_from_deployed(
                    deployed,
                    *contract_idx,
                    &sid_map,
                    &source_cache,
                    &mut source_map_lines,
                    &mut pc_map_cache,
                );
            }
        }

        for counts in &matched_counts {
            let Some((contract_idx, is_initcode)) = codehash_match.get(&counts.contract_id) else {
                continue;
            };
            let Some(pc_map) = pc_map_cache.get(&(*contract_idx, *is_initcode)) else {
                continue;
            };
            let contract = &contracts[*contract_idx];

            debug!(
                "matched contract: {}:{} (bytecode len={}, initcode={})",
                contract.path.display(),
                contract.name,
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
                if entry.source_index < 0 {
                    continue;
                }
                let idx = entry.source_index as usize;
                let Some(path) = sid_map.get(&idx) else {
                    continue;
                };

                let Some(content) = source_cache.get(path) else {
                    continue;
                };
                if content.is_empty() {
                    continue;
                }

                let line = offset_to_line(content, entry.offset);

                source_map_lines
                    // checkrs: allow(clone_in_loops)
                    .entry(path.clone())
                    .or_default()
                    .insert(line);

                // Update line hits (binary: 1 = hit, 0 = not hit).
                // checkrs: allow(clone_in_loops)
                let file_hits = line_hits.entry(path.clone()).or_default();
                file_hits.entry(line).or_insert(1);
            }
        }

        // 6. Determine executable lines.
        //
        // A line is executable when:
        //   1. It appears in source_map_lines (has a source map entry).
        //   2. It is not a close-bracket line (trimmed == "}").
        //   3. It is not empty (trimmed.is_empty()).
        //   4. It is not a contract/interface/library definition line.
        let contract_def_lines =
            collect_contract_definition_lines(&output, &sid_map, &source_cache);

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

        // 7. Collect function coverage from AST.
        let file_functions = collect_function_coverage(
            &output,
            &sid_map,
            &source_cache,
            &source_map_lines,
            &matched_counts,
            &pc_map_cache,
            &codehash_match,
        );

        // Ensure every function start line has a DA entry whose value
        // is 1 if the function was hit at least once, 0 otherwise.
        let mut file_functions_out: HashMap<PathBuf, Vec<FunctionCoverage>> = HashMap::new();
        for (path, functions) in file_functions {
            // checkrs: allow(clone_in_loops)
            let file_lines = executable_line_hits.entry(path.clone()).or_default();

            for func in &functions {
                file_lines.insert(func.line, u64::from(func.hits > 0));
            }
            file_functions_out.insert(path, functions);
        }

        // Ensure all resolved source files appear in the report.
        for path in &all_files {
            // checkrs: allow(clone_in_loops)
            executable_line_hits.entry(path.clone()).or_default();
        }

        // 8. Assemble the report.
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
