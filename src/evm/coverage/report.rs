//! Coverage report generation for the fuzzer.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use alloy_json_abi::Function;
use alloy_primitives::{B256, keccak256};
use anyhow::{Context, Result};
use revm::bytecode::Bytecode;
use revm::primitives::Bytes;

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
        let deployed = artifact.deployed_bytecode()?;
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
struct SourceFile {
    content: String,
    line_offsets: Vec<usize>,
}

impl SourceFile {
    fn load(project_path: impl AsRef<Path>, relative_path: impl AsRef<Path>) -> Result<Self> {
        let path = project_path.as_ref().join(relative_path.as_ref());
        let content = fs::read_to_string(&path)?;
        let line_offsets = build_line_offsets(&content);
        Ok(Self {
            content,
            line_offsets,
        })
    }

    fn offset_to_line(&self, offset: usize) -> usize {
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
struct LineHit {
    line: usize,
    hit_count: u64,
    content: String,
}

/// A per-function coverage summary.
#[derive(Debug, Clone)]
struct FunctionCoverage {
    file_path: PathBuf,
    symbol: String,
    start_line: usize,
    end_line: usize,
    total_lines: usize,
    covered_lines: usize,
    line_hits: Vec<LineHit>,
}

/// Write the coverage report for a fuzzing campaign.
///
/// Returns the path to the coverage directory.
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
    let contract_name = &artifact_id.name;

    let coverage_dir = project_path
        .join("raptor")
        .join("campaigns")
        .join(campaign_id)
        .join("coverage");
    fs::create_dir_all(&coverage_dir)?;

    // Find the target artifact.
    let Some(target_artifact) = build_artifacts.get(artifact_id) else {
        return Err(anyhow::anyhow!(
            "target artifact not found in build artifacts"
        ));
    };

    // Match runtime code with artifact.
    let Some(artifact) = find_artifact_by_runtime_code(runtime_code, build_artifacts) else {
        return Err(anyhow::anyhow!(
            "could not match runtime code to any artifact"
        ));
    };

    let deployed = artifact
        .deployed_bytecode()
        .context("artifact has no deployed bytecode")?;
    let source_map = parse_source_map(&deployed.source_map);
    let bytecode = Bytecode::new_legacy(runtime_code.clone());
    let pc_to_source = build_pc_to_source_map(&bytecode, &source_map);

    let contract_id = B256::from(keccak256(runtime_code));
    let raw_counts = shared_coverage
        .raw_edge_counts(&contract_id)
        .unwrap_or_else(|| vec![0; pc_to_source.len()]);

    // Build source index mapping.
    let source_index = load_source_index(project_path)?;

    // Load source files and map hits to lines.
    let mut source_files: HashMap<PathBuf, SourceFile> = HashMap::new();
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

        let file = source_files.entry(source_path.clone()).or_insert_with(|| {
            SourceFile::load(project_path, &source_path).unwrap_or_else(|_| SourceFile {
                content: String::new(),
                line_offsets: Vec::new(),
            })
        });

        let line = file.offset_to_line(entry.offset);
        *line_hits.entry((source_path, line)).or_insert(0) += raw_count;
    }

    // Build function coverage reports.
    let mut function_coverages: Vec<FunctionCoverage> = Vec::new();
    let all_functions: Vec<&Function> = target_contract
        .target_functions
        .iter()
        .chain(target_contract.invariant_functions.iter())
        .collect();

    for func in all_functions {
        let Some(func_def) = find_function_definition(target_artifact.ast(), contract_name, func)
        else {
            continue;
        };

        let source_path = source_index
            .get(&func_def.src.source_index)
            .cloned()
            .unwrap_or_else(|| artifact.id().path.to_path_buf());

        let file = source_files.entry(source_path.clone()).or_insert_with(|| {
            SourceFile::load(project_path, &source_path).unwrap_or_else(|_| SourceFile {
                content: String::new(),
                line_offsets: Vec::new(),
            })
        });

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

    // Write per-function reports.
    let mut summary_total_lines = 0;
    let mut summary_covered_lines = 0;

    for func_cov in &function_coverages {
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
        fs::write(&file_path, content)?;

        summary_total_lines += func_cov.total_lines;
        summary_covered_lines += func_cov.covered_lines;
    }

    // Write summary.
    let summary_path = coverage_dir.join("summary.txt");
    let mut summary = String::new();
    let summary_uncovered = summary_total_lines.saturating_sub(summary_covered_lines);
    let summary_pct = if summary_total_lines > 0 {
        (summary_covered_lines as f64 / summary_total_lines as f64) * 100.0
    } else {
        0.0
    };

    summary.push_str("Global stats:\n\n");
    summary.push_str(&format!("total lines: {}\n", summary_total_lines));
    summary.push_str(&format!("total covered lines: {}\n", summary_covered_lines));
    summary.push_str(&format!("total uncovered lines: {}\n", summary_uncovered));
    summary.push_str(&format!("coverage: {:.2}%\n", summary_pct));
    summary.push_str("\nBreakdown stats:\n\n");

    for func_cov in &function_coverages {
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

    fs::write(&summary_path, summary)?;

    Ok(coverage_dir)
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
