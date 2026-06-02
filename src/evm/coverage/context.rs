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

fn build_bytecode_entry(
    bytecode: &ArtifactBytecode,
    id: &ArtifactId,
    is_library: bool,
) -> Option<BytecodeEntry> {
    let code = parse_bytecode_with_placeholders(&bytecode.object, &bytecode.link_references);
    if code.is_empty() {
        return None;
    }
    let mut positions = collect_link_positions(&bytecode.link_references);
    // Library runtime code embeds the library's own address at the start of the
    // PUSH20 operand (PUSH20 <address> ADDRESS EQ ...). The compiler writes 0x0
    // in the artifact, so the deployed bytecode differs at these 20 bytes. We mask
    // them out so that linked and unlinked library bytecodes match.
    if is_library && code.first() == Some(&0x73) {
        positions.push((1, 20));
    }
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

    /// Configure this context for a specific target artifact.
    ///
    /// Loads the source index and source files for the artifact's compilation
    /// unit so that the reporter can resolve bytecode hits back to source
    /// lines.
    pub fn with_target_artifact(self, target_artifact_id: &ArtifactId) -> Result<Self> {
        let artifact = self
            .artifacts
            .get(target_artifact_id)
            .with_context(|| "target artifact not found")?;

        // Load the source index for this specific artifact's compilation unit.
        // Foundry incremental builds create multiple build-info files; we must
        // use the one that matches the artifact's compilation unit so that
        // source IDs in the bytecode source map resolve to the correct files.
        let build_info_sources = BuildInfo::load_source_index_for_artifact(
            &self.project_path,
            target_artifact_id,
            artifact.source_id(),
        )?;
        let mut ctx = self;
        ctx.source_index.clear();
        for (idx, path) in build_info_sources {
            ctx.source_index.insert(idx, path);
        }

        for path in ctx.source_index.values().cloned() {
            let full_path = ctx.project_path.join(&path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                ctx.source_files.insert(path, SourceFile::new(content));
            }
        }

        ctx.target_artifact = Some(target_artifact_id.clone());
        Ok(ctx)
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

    /// Look up an artifact by the keccak256 hash of its runtime bytecode.
    pub fn resolve_artifact_by_hash(&self, hash: &B256) -> Option<&Artifact> {
        for entry in &self.runtime_entries {
            if *hash == entry.base_hash {
                return self.artifacts.get(&entry.id);
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

        let mut contracts_to_search = vec![contract];
        for base_id in &contract.linearized_base_contracts {
            if let Some(base_contract) = self.find_contract_by_id(*base_id) {
                contracts_to_search.push(base_contract);
            }
        }

        for contract in contracts_to_search {
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
        }
        None
    }

    /// Collect all contract symbols into a map keyed by declaration ID.
    ///
    /// Symbols are resolved from the contract and all of its base contracts
    /// (via `linearized_base_contracts`) so that inherited functions and state
    /// variables are included in coverage reports. Additionally, symbols from
    /// all other loaded artifacts are included so that external contract calls
    /// and library functions can be resolved in coverage reports.
    pub fn resolve_contract_symbols<'a>(
        &'a self,
        artifact: &'a Artifact,
        contract_name: &str,
    ) -> Option<HashMap<i64, &'a solc::ast::ContractDefinitionNode>> {
        let ast = artifact.ast();
        let contract = get_contract_definition(ast, contract_name).ok()?;
        let mut map = HashMap::new();
        for base_id in &contract.linearized_base_contracts {
            let base_contract = self.find_contract_by_id(*base_id)?;
            for node in &base_contract.nodes {
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
        }
        // Include symbols from all other artifacts so that external calls and
        // library functions can be traced.
        for other_artifact in self.artifacts.values() {
            let other_ast = other_artifact.ast();
            for node in &other_ast.nodes {
                let solc::ast::SourceUnitNode::ContractDefinition(contract) = node else {
                    continue;
                };
                for inner_node in &contract.nodes {
                    match inner_node {
                        solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
                            map.insert(func.id, inner_node);
                        }
                        solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
                            map.insert(var.id, inner_node);
                        }
                        _ => {}
                    }
                }
            }
        }
        Some(map)
    }

    /// Find a contract definition by its ID across all loaded artifacts.
    fn find_contract_by_id(&self, id: i64) -> Option<&solc::ast::ContractDefinition> {
        for artifact in self.artifacts.values() {
            let ast = artifact.ast();
            for node in &ast.nodes {
                if let solc::ast::SourceUnitNode::ContractDefinition(def) = node
                    && def.id == id
                {
                    return Some(def);
                }
            }
        }
        None
    }

    /// Find all implemented contract nodes (functions or public state variables)
    /// that match the given function selector across all loaded artifacts.
    ///
    /// This is used to resolve interface references to their actual
    /// implementations when the coverage reporter encounters an external call
    /// made through an interface.
    pub fn find_implementations_by_selector<'a>(
        &'a self,
        selector: &str,
    ) -> Vec<&'a solc::ast::ContractDefinitionNode> {
        let mut results = Vec::new();
        let target = selector.trim_start_matches("0x").to_lowercase();
        for artifact in self.artifacts.values() {
            let ast = artifact.ast();
            for node in &ast.nodes {
                let solc::ast::SourceUnitNode::ContractDefinition(contract) = node else {
                    continue;
                };
                for inner_node in &contract.nodes {
                    match inner_node {
                        solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
                            if func.implemented
                                && let Some(ref sel) = func.function_selector
                                && sel.trim_start_matches("0x").to_lowercase() == target
                            {
                                results.push(inner_node);
                            }
                        }
                        solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
                            if var.state_variable
                                && let Some(ref sel) = var.function_selector
                                && sel.trim_start_matches("0x").to_lowercase() == target
                            {
                                results.push(inner_node);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        results
    }

    /// Build a line-hit map from the shared coverage across all contracts.
    ///
    /// Returns a map of `(source_path, line_number) -> hit_count`.
    pub fn build_line_hits(
        &self,
        shared_coverage: &SharedCoverage,
    ) -> HashMap<(PathBuf, usize), u64> {
        let mut line_hits: HashMap<(PathBuf, usize), u64> = HashMap::new();
        let empty_source = SourceFile {
            content: String::new(),
            line_offsets: Vec::new(),
        };

        for counts in shared_coverage.all_raw_edge_counts_with_bytecodes() {
            let Some(artifact) = self
                .resolve_artifact_by_runtime_code(&Bytes::from(counts.bytecode))
                .or_else(|| self.resolve_artifact_by_hash(&counts.contract_id))
            else {
                continue;
            };
            let raw_counts = counts.raw_edges;
            let Some(deployed) = artifact.deployed_bytecode() else {
                continue;
            };
            let source_map = parse_source_map(&deployed.source_map);
            let code =
                parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
            let bytecode = Bytecode::new_legacy(Bytes::from(code));
            let pc_to_source = build_pc_to_source_map(&bytecode, &source_map);

            for (pc, entry) in pc_to_source.iter().enumerate() {
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
        }

        line_hits
    }

    /// Build a set of all executable lines across all contracts.
    ///
    /// Returns a set of `(source_path, line_number)` pairs for every line that
    /// maps to at least one program counter in any runtime bytecode.
    pub fn build_executable_lines(&self) -> HashSet<(PathBuf, usize)> {
        let mut lines: HashSet<(PathBuf, usize)> = HashSet::new();

        let empty_source = SourceFile {
            content: String::new(),
            line_offsets: Vec::new(),
        };

        for entry in &self.runtime_entries {
            let Some(artifact) = self.artifacts.get(&entry.id) else {
                continue;
            };
            let Some(deployed) = artifact.deployed_bytecode() else {
                continue;
            };
            let source_map = parse_source_map(&deployed.source_map);
            let code =
                parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
            let bytecode = Bytecode::new_legacy(Bytes::from(code));
            let pc_to_source = build_pc_to_source_map(&bytecode, &source_map);

            for entry in &pc_to_source {
                let Some(entry) = entry else { continue };
                let Some(source_path) = self.source_index.get(&entry.source_index) else {
                    continue;
                };
                let source_path = source_path.to_path_buf();
                let file = self.source_files.get(&source_path).unwrap_or(&empty_source);
                let line = file.offset_to_line(entry.offset);
                lines.insert((source_path, line));
            }
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
            let is_library = matches!(artifact, Artifact::Library(_));
            let runtime_entry = artifact
                .deployed_bytecode()
                .and_then(|b| build_bytecode_entry(b, &id, is_library));
            let initcode_entry = artifact
                .bytecode()
                .and_then(|b| build_bytecode_entry(b, &id, is_library));
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
        let ctx = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(target.id())
            .unwrap();
        assert!(ctx.resolve_source_file("src/TargetContract.sol").is_some());
    }
}
