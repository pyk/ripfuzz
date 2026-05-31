//! Coverage report generation for the fuzzer.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_json_abi::Function;
use alloy_primitives::{B256, keccak256};
use anyhow::{Context, Result};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;
use tracing::{debug, trace};

use crate::evm::coverage::shared::SharedCoverage;
use crate::evm::coverage::source_map::{SourceMapEntry, parse_source_map};
use crate::foundry::{Artifact, ArtifactId, LinkReferences, get_contract_definition};

/// Collect link-reference positions from a `linkReferences` map.
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

/// Zero out the bytes at the given positions in a mutable buffer.
fn zero_out_positions(buf: &mut [u8], positions: &[(usize, usize)]) {
    for (start, len) in positions {
        for i in *start..*start + *len {
            if i < buf.len() {
                buf[i] = 0;
            }
        }
    }
}

/// Parse a bytecode object string, replacing library placeholders at the
/// given link-reference positions with zero bytes.
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

/// Find the artifact that matches the given runtime bytecode.
fn find_artifact_by_runtime_code<'a>(
    runtime_code: &Bytes,
    artifacts: &'a HashMap<ArtifactId, Artifact>,
) -> Option<&'a Artifact> {
    for artifact in artifacts.values() {
        let Some(deployed) = artifact.deployed_bytecode() else {
            continue;
        };
        let artifact_code =
            parse_bytecode_with_placeholders(&deployed.object, &deployed.link_references);
        if artifact_code.is_empty() {
            continue;
        }
        let positions = collect_link_positions(&deployed.link_references);
        let mut masked_runtime = runtime_code.to_vec();
        zero_out_positions(&mut masked_runtime, &positions);
        let mut masked_artifact = artifact_code;
        zero_out_positions(&mut masked_artifact, &positions);
        if keccak256(&masked_runtime) == keccak256(&masked_artifact) {
            return Some(artifact);
        }
    }
    None
}

/// A single source file with line offsets.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub content: String,
    pub line_offsets: Vec<usize>,
}

impl SourceFile {
    pub fn load(project_path: impl AsRef<Path>, relative_path: impl AsRef<Path>) -> Result<Self> {
        let path = project_path.as_ref().join(relative_path.as_ref());
        let content = fs::read_to_string(&path)?;
        let line_offsets = build_line_offsets(&content);
        Ok(Self {
            content,
            line_offsets,
        })
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

/// Build a mapping from program counter to source map entry for a bytecode.
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

/// Find the AST [`FunctionDefinition`] that matches a target ABI function.
fn find_function_definition<'a>(
    ast: &'a solc::ast::SourceUnit,
    contract_name: &str,
    target_func: &Function,
) -> Option<&'a solc::ast::FunctionDefinition> {
    let contract = get_contract_definition(ast, contract_name).ok()?;
    let target_selector = hex::encode(target_func.selector());
    for node in &contract.nodes {
        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node
            && let Some(ref sel) = func.function_selector
            && sel.trim_start_matches("0x").to_lowercase() == target_selector.to_lowercase()
        {
            return Some(func);
        }
    }
    None
}

/// A line hit record for a single source line.
#[derive(Debug, Clone)]
pub struct LineHit {
    pub line: usize,
    pub hit_count: u64,
    pub content: String,
}

/// A per-function coverage summary.
#[derive(Debug, Clone)]
pub struct FunctionCoverage {
    pub file_path: PathBuf,
    pub symbol: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub covered_lines: usize,
    pub line_hits: Vec<LineHit>,
}

/// Write the coverage report for a fuzzing campaign.
///
/// Returns the path to the coverage directory.
/// A coverage report generated for a fuzzing campaign.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    pub function_coverages: Vec<FunctionCoverage>,
    pub summary_total_lines: usize,
    pub summary_covered_lines: usize,
}

impl CoverageReport {
    /// Build a coverage report from collected coverage data and build artifacts.
    ///
    /// This is a pure-logic operation: it does not read from or write to the
    /// filesystem.
    pub fn build(
        shared_coverage: &SharedCoverage,
        target_contract: &crate::evm::Contract,
        build_artifacts: &HashMap<ArtifactId, Artifact>,
        runtime_code: &Bytes,
        source_index: &HashMap<usize, PathBuf>,
        source_files: &HashMap<PathBuf, SourceFile>,
    ) -> Result<Self> {
        let artifact_id = &target_contract.artifact_id;
        let contract_name = &artifact_id.name;

        // Find the target artifact.
        let Some(target_artifact) = build_artifacts.get(artifact_id) else {
            debug!(?artifact_id, "target artifact not found in build artifacts");
            return Err(anyhow::anyhow!(
                "target artifact not found in build artifacts"
            ));
        };
        trace!("found target artifact");

        // Match runtime code with artifact.
        let Some(artifact) = find_artifact_by_runtime_code(runtime_code, build_artifacts) else {
            debug!(
                runtime_code_len = runtime_code.len(),
                "could not match runtime code to any artifact"
            );
            return Err(anyhow::anyhow!(
                "could not match runtime code to any artifact"
            ));
        };
        trace!(artifact_id = ?artifact.id(), "matched artifact by runtime code");

        let deployed = artifact
            .deployed_bytecode()
            .context("artifact has no deployed bytecode")?;
        trace!(
            source_map_len = deployed.source_map.len(),
            "loaded deployed bytecode"
        );

        let source_map = parse_source_map(&deployed.source_map);
        trace!(source_map_entries = source_map.len(), "parsed source map");

        let bytecode = Bytecode::new_legacy(runtime_code.clone());
        let pc_to_source = build_pc_to_source_map(&bytecode, &source_map);
        trace!(pc_count = pc_to_source.len(), "built pc to source map");

        let contract_id = B256::from(keccak256(runtime_code));
        let raw_counts = shared_coverage
            .raw_edge_counts(&contract_id)
            .unwrap_or_else(|| vec![0; pc_to_source.len()]);
        trace!(
            total_hits = raw_counts.iter().sum::<u64>(),
            "loaded raw edge counts"
        );

        trace!(source_index_len = source_index.len(), "loaded source index");

        let empty_source = SourceFile {
            content: String::new(),
            line_offsets: Vec::new(),
        };

        let mut line_hits: HashMap<(PathBuf, usize), u64> = HashMap::new();

        for (pc, entry) in pc_to_source.iter().enumerate() {
            let Some(entry) = entry else { continue };
            let raw_count = raw_counts.get(pc).copied().unwrap_or(0);
            if raw_count == 0 {
                continue;
            }

            let source_path = source_index
                .get(&entry.source_index)
                .cloned()
                .unwrap_or_else(|| artifact.id().path.to_path_buf());

            let file = source_files.get(&source_path).unwrap_or(&empty_source);

            let line = file.offset_to_line(entry.offset);
            *line_hits.entry((source_path, line)).or_insert(0) += raw_count;
        }
        trace!(line_hits_count = line_hits.len(), "mapped hits to lines");

        // Build function coverage reports.
        let mut function_coverages: Vec<FunctionCoverage> = Vec::new();
        let all_functions: Vec<&Function> = target_contract
            .target_functions
            .iter()
            .chain(target_contract.invariant_functions.iter())
            .collect();

        for func in all_functions {
            let Some(func_def) =
                find_function_definition(target_artifact.ast(), contract_name, func)
            else {
                trace!(
                    func = func.signature(),
                    "function definition not found in AST"
                );
                continue;
            };
            trace!(func = func.signature(), "found function definition in AST");

            let source_path = source_index
                .get(&func_def.src.source_index)
                .cloned()
                .unwrap_or_else(|| artifact.id().path.to_path_buf());

            let file = source_files.get(&source_path).unwrap_or(&empty_source);

            let start_line = file.offset_to_line(func_def.src.offset);
            let end_line = file.offset_to_line(func_def.src.offset + func_def.src.length);
            let total_lines = end_line.saturating_sub(start_line) + 1;

            let mut covered_lines = 0;
            let mut hits = Vec::new();
            for line in start_line..=end_line {
                let hit_count = line_hits
                    .get(&(source_path.clone(), line))
                    .copied()
                    .unwrap_or(0);
                if hit_count > 0 {
                    covered_lines += 1;
                }
                let content: String = file
                    .content
                    .lines()
                    .nth(line.saturating_sub(1))
                    .unwrap_or("")
                    .into();
                hits.push(LineHit {
                    line,
                    hit_count,
                    content,
                });
            }

            function_coverages.push(FunctionCoverage {
                file_path: source_path.clone(),
                symbol: func.signature(),
                start_line,
                end_line,
                total_lines,
                covered_lines,
                line_hits: hits,
            });
        }
        debug!(
            function_count = function_coverages.len(),
            "built function coverages"
        );

        let summary_total_lines = function_coverages.iter().map(|f| f.total_lines).sum();
        let summary_covered_lines = function_coverages.iter().map(|f| f.covered_lines).sum();

        Ok(Self {
            function_coverages,
            summary_total_lines,
            summary_covered_lines,
        })
    }

    /// Write the report to the project directory.
    ///
    /// This is the I/O half of the operation: it creates directories and writes
    /// files.
    pub fn write(&self, project_path: impl AsRef<Path>, campaign_id: &str) -> Result<PathBuf> {
        let project_path = project_path.as_ref();
        let coverage_dir = project_path
            .join("raptor")
            .join("campaigns")
            .join(campaign_id)
            .join("coverage");
        fs::create_dir_all(&coverage_dir)?;
        trace!(?coverage_dir, "created coverage directory");

        for func_cov in &self.function_coverages {
            let file_name = sanitize_function_name(&func_cov.symbol);
            let file_path = coverage_dir.join(format!("{file_name}.txt"));
            let mut content = String::new();

            let uncovered = func_cov.total_lines.saturating_sub(func_cov.covered_lines);
            let pct = if func_cov.total_lines > 0 {
                (func_cov.covered_lines as f64 / func_cov.total_lines as f64) * 100.0
            } else {
                0.0
            };

            content.push_str("Function stats:\n\n");
            content.push_str(&format!("total lines: {}\n", func_cov.total_lines));
            content.push_str(&format!(
                "total covered lines: {}\n",
                func_cov.covered_lines
            ));
            content.push_str(&format!("total uncovered lines: {}\n", uncovered));
            content.push_str(&format!("coverage: {:.2}%\n", pct));
            content.push_str("\nBreakdown stats:\n\n");
            content.push_str(&format!(
                "path: {}#L{}-L{}\n",
                func_cov.file_path.display(),
                func_cov.start_line,
                func_cov.end_line,
            ));
            content.push_str(&format!("symbol: {}\n", func_cov.symbol));
            content.push_str("line hits:\n\n");
            for hit in &func_cov.line_hits {
                content.push_str(&format!(
                    "{:4} | {:4} |{}\n",
                    hit.line, hit.hit_count, hit.content,
                ));
            }
            trace!(?file_path, "writing function coverage report");
            fs::write(&file_path, content)?;
        }

        let summary_path = coverage_dir.join("summary.txt");
        let mut summary = String::new();
        let summary_uncovered = self
            .summary_total_lines
            .saturating_sub(self.summary_covered_lines);
        let summary_pct = if self.summary_total_lines > 0 {
            (self.summary_covered_lines as f64 / self.summary_total_lines as f64) * 100.0
        } else {
            0.0
        };

        summary.push_str("Global stats:\n\n");
        summary.push_str(&format!("total lines: {}\n", self.summary_total_lines));
        summary.push_str(&format!(
            "total covered lines: {}\n",
            self.summary_covered_lines
        ));
        summary.push_str(&format!("total uncovered lines: {}\n", summary_uncovered));
        summary.push_str(&format!("coverage: {:.2}%\n", summary_pct));
        summary.push_str("\nBreakdown stats:\n\n");

        for func_cov in &self.function_coverages {
            let func_pct = if func_cov.total_lines > 0 {
                (func_cov.covered_lines as f64 / func_cov.total_lines as f64) * 100.0
            } else {
                0.0
            };
            let file_name = sanitize_function_name(&func_cov.symbol);
            summary.push_str(&format!(
                "file path: raptor/campaigns/{}/coverage/{}.txt\n",
                campaign_id, file_name,
            ));
            summary.push_str(&format!("total lines: {}\n", func_cov.total_lines));
            summary.push_str(&format!(
                "coverage percentage for that file: {:.2}%\n",
                func_pct
            ));
            summary.push('\n');
        }

        trace!(?summary_path, "writing summary report");
        fs::write(&summary_path, summary)?;
        debug!("coverage report written successfully");

        Ok(coverage_dir)
    }
}

/// Convenience wrapper that loads source files, builds the report, and writes it.
///
/// For full control over I/O, use [`CoverageReport::build`] and
/// [`CoverageReport::write`] directly.
pub fn write_coverage_report(
    project_path: impl AsRef<Path>,
    campaign_id: &str,
    shared_coverage: &SharedCoverage,
    target_contract: &crate::evm::Contract,
    build_artifacts: &HashMap<ArtifactId, Artifact>,
    runtime_code: &Bytes,
) -> Result<PathBuf> {
    let project_path = project_path.as_ref();
    let artifact_id = &target_contract.artifact_id;

    debug!(
        ?project_path,
        ?campaign_id,
        ?artifact_id,
        "writing coverage report"
    );

    let source_index = load_source_index(project_path)?;

    // Pre-load all source files that may be referenced.
    let mut source_files = HashMap::new();
    for path in source_index.values().cloned() {
        if let Ok(file) = SourceFile::load(project_path, &path) {
            source_files.insert(path, file);
        }
    }
    let artifact_path = artifact_id.path.clone();
    if !source_files.contains_key(&artifact_path)
        && let Ok(file) = SourceFile::load(project_path, &artifact_path)
    {
        source_files.insert(artifact_path, file);
    }

    let report = CoverageReport::build(
        shared_coverage,
        target_contract,
        build_artifacts,
        runtime_code,
        &source_index,
        &source_files,
    )?;
    report.write(project_path, campaign_id)
}

/// Load the source index mapping from build-info files.
fn load_source_index(project_path: impl AsRef<Path>) -> Result<HashMap<usize, PathBuf>> {
    let project_path = project_path.as_ref();
    let build_info_dir = project_path.join("out").join("build-info");
    let mut source_index = HashMap::new();
    if !build_info_dir.exists() {
        return Ok(source_index);
    }
    for entry in fs::read_dir(&build_info_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some("json".as_ref()) {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        let json: serde_json::Value = serde_json::from_str(&content)?;
        let Some(map) = json.get("source_id_to_path").and_then(|v| v.as_object()) else {
            continue;
        };
        for (k, v) in map {
            if let Ok(idx) = k.parse::<usize>()
                && let Some(path_str) = v.as_str()
            {
                source_index.insert(idx, PathBuf::from(path_str));
            }
        }
    }
    Ok(source_index)
}

/// Sanitize a function signature for use as a filename.
///
/// Returns only the function name (e.g. `add` for `add(uint256)`).
fn sanitize_function_name(name: &str) -> String {
    let base = name.split('(').next().unwrap_or(name);
    base.replace(
        |c: char| c.is_ascii_whitespace() || c == '\n' || c == '\r',
        "_",
    )
    .replace("(", "")
    .replace(")", "")
    .replace(",", "")
    .replace(" ", "_")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, Transaction};
    use crate::evm::coverage::SharedCoverage;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    /// Coverage report logic must succeed even when the build artifacts contain
    /// interface/abstract contracts that have no deployed bytecode.
    #[test]
    fn coverage_report_build_with_interface_artifact() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");

        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();
        let runtime_code = deployment.result.output.unwrap_or_default();

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec = chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let build_artifacts = project.load_artifacts().unwrap();
        let source_index = super::load_source_index("fixtures/target-contract-coverage").unwrap();

        let mut source_files = HashMap::new();
        for path in source_index.values().cloned() {
            if let Ok(file) = super::SourceFile::load("fixtures/target-contract-coverage", &path) {
                source_files.insert(path, file);
            }
        }
        let artifact_path = contract.artifact_id.path.clone();
        if !source_files.contains_key(&artifact_path)
            && let Ok(file) =
                super::SourceFile::load("fixtures/target-contract-coverage", &artifact_path)
        {
            source_files.insert(artifact_path, file);
        }

        let report = super::CoverageReport::build(
            &global,
            &contract,
            &build_artifacts,
            &runtime_code,
            &source_index,
            &source_files,
        )
        .unwrap();

        assert!(
            !report.function_coverages.is_empty(),
            "coverage report should contain function coverages even when build artifacts include interfaces"
        );
    }
}
