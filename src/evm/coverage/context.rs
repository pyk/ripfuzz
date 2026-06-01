//! Coverage context for mapping raw bytecode hits back to Solidity source lines.
//!
//! [`CoverageContext`] is the data layer of the coverage reporting pipeline.
//! It collects three kinds of information from one or more Foundry projects:
//!
//! 1. **Build artifacts** [`Artifact`] - compiled contracts, their ASTs, and
//!    bytecode (both initcode and runtime code).
//! 2. **Source files** [`SourceFile`] - original `.sol` source text with line
//!    offset tables for fast byte-offset-to-line conversion.
//! 3. **Source indices** - a map from compiler source IDs to project-relative
//!    paths, extracted from `build-info/*.json`.
//!
//! Once populated, the context is configured for a target contract via
//! [`CoverageContext::with_runtime_code`]. This method hashes the runtime
//! bytecode against the indexed artifacts, identifies the matching contract,
//! and builds a PC-to-source map so that every program counter can be resolved
//! to a `(source_path, line)` pair.
//!
//! The main consumer of this context is [`CoverageReporter`](super::CoverageReporter).
//!
//! The reporter calls [`CoverageContext::build_line_hits`] to translate the
//! raw edge counts stored in [`SharedCoverage`] into a line-hit map, then uses
//! the AST lookup methods (`resolve_function_definition` etc.) to build
//! human-readable per-function reports.
//!
//! In short: `CoverageContext` knows what code was hit and where it lives in
//! source; `CoverageReporter` decides how to present that information.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use alloy_json_abi::Function;
use alloy_primitives::{B256, keccak256};
use anyhow::{Context, Result};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;
use tracing::debug;

use crate::evm::coverage::shared::SharedCoverage;
use crate::evm::coverage::source_map::{SourceMapEntry, parse_source_map};
use crate::foundry::BuildInfo;
use crate::foundry::{
    Artifact, ArtifactBytecode, ArtifactId, LinkReferences, Project, get_contract_definition,
};

// ---------------------------------------------------------------------------
// Source file helpers
// ---------------------------------------------------------------------------

/// A single source file with line offsets.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub content: String,
    pub line_offsets: Vec<usize>,
}

impl SourceFile {
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        let line_offsets = build_line_offsets(&content);
        Self {
            content,
            line_offsets,
        }
    }

    pub fn offset_to_line(&self, offset: usize) -> usize {
        match self.line_offsets.binary_search(&offset) {
            Ok(line) => line + 1,
            Err(line) => line,
        }
    }
}

fn build_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (i, c) in content.char_indices() {
        if c == '\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

// ---------------------------------------------------------------------------
// Bytecode helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BytecodeEntry {
    id: ArtifactId,
    base_hash: B256,
    positions: Vec<(usize, usize)>,
}

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

fn build_pc_to_source_map(
    bytecode: &Bytecode,
    source_map: &[SourceMapEntry],
) -> Vec<Option<SourceMapEntry>> {
    let mut result = vec![None; bytecode.len()];
    let mut iter = bytecode.iter_opcodes();
    let mut opcode_idx = 0;
    while let Some(_opcode) = iter.peek() {
        let pc = iter.position();
        if pc >= bytecode.len() {
            break;
        }
        if let Some(entry) = source_map.get(opcode_idx) {
            result[pc] = Some(*entry);
        }
        iter.next();
        opcode_idx += 1;
    }
    result
}

fn build_bytecode_entry(bytecode: &ArtifactBytecode, id: &ArtifactId) -> Option<BytecodeEntry> {
    let code = parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
    if code.is_empty() {
        return None;
    }
    let positions = collect_link_positions(&bytecode.link_references);
    let mut masked = code;
    zero_out_positions(&mut masked, &positions);
    Some(BytecodeEntry {
        id: ArtifactId::clone(id),
        base_hash: keccak256(&masked),
        positions,
    })
}

// ---------------------------------------------------------------------------
// CoverageContext
// ---------------------------------------------------------------------------

/// Context for building coverage reports.
///
/// Collects build artifacts, source files, and source indices from one or more
/// Foundry projects. Can be configured for a specific target runtime code so
/// that the reporter can resolve bytecode hits back to source lines.
#[derive(Debug, Clone, Default)]
pub struct CoverageContext {
    artifacts: HashMap<ArtifactId, Artifact>,
    runtime_entries: Vec<BytecodeEntry>,
    initcode_entries: Vec<BytecodeEntry>,
    source_files: HashMap<PathBuf, SourceFile>,
    source_index: HashMap<usize, PathBuf>,
    target_artifact: Option<ArtifactId>,
    pc_to_source: Vec<Option<SourceMapEntry>>,
    runtime_code: Option<Bytes>,
    project_path: PathBuf,
}

impl CoverageContext {
    /// Create a new [`CoverageContext`] from a Foundry [`Project`].
    pub fn from_project(project: &Project) -> Result<Self> {
        let mut ctx = Self {
            project_path: project.path.clone(),
            ..Self::default()
        };
        ctx.load_project(project)?;
        Ok(ctx)
    }

    /// Configure this context for a specific target runtime code.
    ///
    /// Resolves the runtime code against the indexed artifacts and builds a
    /// PC-to-source map so that coverage hits can be mapped back to source
    /// lines.
    pub fn with_runtime_code(mut self, runtime_code: &Bytes) -> Result<Self> {
        let artifact = self
            .resolve_artifact_by_runtime_code(runtime_code)
            .with_context(|| "could not match runtime code to any artifact")?;
        let artifact_id = artifact.id().clone();

        let deployed = artifact
            .deployed_bytecode()
            .with_context(|| "artifact has no deployed bytecode")?;
        let source_map = parse_source_map(&deployed.source_map);
        let bytecode = Bytecode::new_legacy(runtime_code.clone());
        let pc_to_source = build_pc_to_source_map(&bytecode, &source_map);

        // Load the source index for this specific artifact's compilation unit.
        // Foundry incremental builds create multiple build-info files; we must
        // use the one that matches the artifact's compilation unit so that
        // source IDs in the bytecode source map resolve to the correct files.
        let build_info_sources = BuildInfo::load_source_index_for_artifact(
            &self.project_path,
            &artifact_id,
            artifact.source_id(),
        )?;
        self.source_index.clear();
        for (idx, path) in build_info_sources {
            self.source_index.insert(idx, path);
        }

        for path in self.source_index.values().cloned() {
            let full_path = self.project_path.join(&path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                self.source_files.insert(path, SourceFile::new(content));
            }
        }

        self.target_artifact = Some(artifact_id);
        self.pc_to_source = pc_to_source;
        self.runtime_code = Some(runtime_code.clone());
        Ok(self)
    }

    /// Look up an artifact by its runtime bytecode.
    pub fn resolve_artifact_by_runtime_code(&self, runtime_code: &Bytes) -> Option<&Artifact> {
        let mut masked = runtime_code.to_vec();
        for entry in &self.runtime_entries {
            zero_out_positions(&mut masked, &entry.positions);
            if keccak256(&masked) == entry.base_hash {
                return self.artifacts.get(&entry.id);
            }
            // Restore masked bytes for the next entry.
            for (start, len) in &entry.positions {
                for i in *start..*start + *len {
                    if i < masked.len() {
                        masked[i] = runtime_code[i];
                    }
                }
            }
        }
        None
    }

    /// Look up a source file by its project-relative path.
    pub fn resolve_source_file(&self, path: impl AsRef<Path>) -> Option<&SourceFile> {
        self.source_files.get(path.as_ref())
    }

    /// Look up a source file path by its source index.
    pub fn resolve_source_index(&self, index: usize) -> Option<&PathBuf> {
        self.source_index.get(&index)
    }

    /// Find the AST function definition that matches a target ABI function.
    pub fn resolve_function_definition<'a>(
        &'a self,
        artifact: &'a Artifact,
        contract_name: &str,
        target_func: &Function,
    ) -> Option<&'a solc::ast::FunctionDefinition> {
        let ast = artifact.ast();
        let contract = get_contract_definition(ast, contract_name).ok()?;
        let target_selector = hex::encode(target_func.selector());
        for node in &contract.nodes {
            let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node else {
                continue;
            };
            let Some(ref sel) = func.function_selector else {
                continue;
            };
            if sel.trim_start_matches("0x").to_lowercase() == target_selector.to_lowercase() {
                return Some(func);
            }
        }
        None
    }

    /// Collect all contract symbols into a map keyed by declaration ID.
    pub fn resolve_contract_symbols<'a>(
        &'a self,
        artifact: &'a Artifact,
        contract_name: &str,
    ) -> Option<HashMap<i64, &'a solc::ast::ContractDefinitionNode>> {
        let ast = artifact.ast();
        let contract = get_contract_definition(ast, contract_name).ok()?;
        let mut map = HashMap::new();
        for node in &contract.nodes {
            match node {
                solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
                    map.insert(func.id, node);
                }
                solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
                    map.insert(var.id, node);
                }
                _ => {}
            }
        }
        Some(map)
    }

    /// Build a line-hit map from the shared coverage and the configured runtime
    /// code.
    ///
    /// Returns a map of `(source_path, line_number) -> hit_count`.
    pub fn build_line_hits(
        &self,
        shared_coverage: &SharedCoverage,
    ) -> HashMap<(PathBuf, usize), u64> {
        let mut line_hits: HashMap<(PathBuf, usize), u64> = HashMap::new();
        let Some(runtime_code) = &self.runtime_code else {
            return line_hits;
        };
        let contract_id = B256::from(keccak256(runtime_code));
        let raw_counts = shared_coverage
            .raw_edge_counts(&contract_id)
            .unwrap_or_else(|| vec![0; self.pc_to_source.len()]);

        let empty_source = SourceFile {
            content: String::new(),
            line_offsets: Vec::new(),
        };

        for (pc, entry) in self.pc_to_source.iter().enumerate() {
            let Some(entry) = entry else { continue };
            let raw_count = raw_counts.get(pc).copied().unwrap_or(0);
            if raw_count == 0 {
                continue;
            }
            let Some(source_path) = self.source_index.get(&entry.source_index) else {
                continue;
            };
            let source_path = source_path.to_path_buf();
            let file = self.source_files.get(&source_path).unwrap_or(&empty_source);
            let line = file.offset_to_line(entry.offset);
            let current = line_hits.entry((source_path, line)).or_insert(0);
            *current = (*current).max(raw_count);
        }

        line_hits
    }

    /// Build a set of all executable lines from the PC-to-source map.
    ///
    /// Returns a set of `(source_path, line_number)` pairs for every line that
    /// maps to at least one program counter in the runtime bytecode.
    pub fn build_executable_lines(&self) -> HashSet<(PathBuf, usize)> {
        let mut lines: HashSet<(PathBuf, usize)> = HashSet::new();
        let Some(runtime_code) = &self.runtime_code else {
            return lines;
        };
        let _contract_id = B256::from(keccak256(runtime_code));

        let empty_source = SourceFile {
            content: String::new(),
            line_offsets: Vec::new(),
        };

        for entry in &self.pc_to_source {
            let Some(entry) = entry else { continue };
            let Some(source_path) = self.source_index.get(&entry.source_index) else {
                continue;
            };
            let source_path = source_path.to_path_buf();
            let file = self.source_files.get(&source_path).unwrap_or(&empty_source);
            let line = file.offset_to_line(entry.offset);
            lines.insert((source_path, line));
        }

        lines
    }

    /// Return the target artifact if one has been configured via
    /// [`with_runtime_code`](Self::with_runtime_code).
    pub fn target_artifact(&self) -> Option<&Artifact> {
        self.target_artifact
            .as_ref()
            .and_then(|id| self.artifacts.get(id))
    }

    fn load_project(&mut self, project: &Project) -> Result<()> {
        let artifacts = project.load_artifacts()?;
        for (id, artifact) in artifacts {
            let runtime_entry = artifact
                .deployed_bytecode()
                .and_then(|b| build_bytecode_entry(b, &id));
            let initcode_entry = artifact
                .bytecode()
                .and_then(|b| build_bytecode_entry(b, &id));
            if let Some(entry) = runtime_entry {
                self.runtime_entries.push(entry);
            }
            if let Some(entry) = initcode_entry {
                self.initcode_entries.push(entry);
            }
            self.artifacts.insert(id, artifact);
        }

        debug!(project = %project.path.display(), "loaded coverage context");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::foundry;

    use super::*;

    #[test]
    fn context_from_project() {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let target = artifacts
            .get(&ArtifactId::try_from("src/TargetContract.sol:TargetContract").unwrap())
            .unwrap();
        let runtime_code = target.deployed_bytecode().unwrap().object.parse().unwrap();
        let ctx = CoverageContext::from_project(&project)
            .unwrap()
            .with_runtime_code(&runtime_code)
            .unwrap();
        assert!(ctx.resolve_source_file("src/TargetContract.sol").is_some());
    }
}
