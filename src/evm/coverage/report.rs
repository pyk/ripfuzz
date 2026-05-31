//! Coverage report generation for the fuzzer.

use std::collections::{HashMap, HashSet};
use std::fmt;
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

/// Build a Solidity signature from an AST function definition.
///
/// The output matches the ABI signature format: `name(param1,param2,…)`
/// without return types.
fn build_signature_from_ast(func: &solc::ast::FunctionDefinition) -> String {
    let mut params = String::new();
    for (i, p) in func.parameters.parameters.iter().enumerate() {
        if i > 0 {
            params.push(',');
        }
        params.push_str(
            p.type_descriptions
                .type_string
                .as_deref()
                .unwrap_or("unknown"),
        );
    }
    let name = &func.name;
    format!("{}({})", name, params)
}

/// Collect referenced declaration IDs from a function call expression.
fn collect_references_from_function_call_expression(
    expr: &solc::ast::FunctionCallExpression,
) -> Vec<i64> {
    let mut refs = Vec::new();
    match expr {
        solc::ast::FunctionCallExpression::Identifier(id) => {
            if let Some(rid) = id.referenced_declaration {
                refs.push(rid);
            }
        }
        solc::ast::FunctionCallExpression::MemberAccess(member) => {
            if let Some(rid) = member.referenced_declaration {
                refs.push(rid);
            }
        }
        solc::ast::FunctionCallExpression::FunctionCall(call) => {
            refs.extend(collect_references_from_function_call(call));
        }
        solc::ast::FunctionCallExpression::FunctionCallOptions(options) => {
            refs.extend(collect_references_from_expression(&options.expression));
        }
        _ => {}
    }
    refs
}

/// Collect referenced declaration IDs from an expression.
fn collect_references_from_expression(expr: &solc::ast::Expression) -> Vec<i64> {
    let mut refs = Vec::new();
    match expr {
        solc::ast::Expression::Identifier(id) => {
            if let Some(rid) = id.referenced_declaration {
                refs.push(rid);
            }
        }
        solc::ast::Expression::MemberAccess(member) => {
            if let Some(rid) = member.referenced_declaration {
                refs.push(rid);
            }
            refs.extend(collect_references_from_expression(&member.expression));
        }
        solc::ast::Expression::FunctionCall(call) => {
            refs.extend(collect_references_from_function_call(call));
        }
        solc::ast::Expression::Assignment(assignment) => {
            refs.extend(collect_references_from_expression(
                &assignment.left_hand_side,
            ));
            refs.extend(collect_references_from_expression(
                &assignment.right_hand_side,
            ));
        }
        solc::ast::Expression::BinaryOperation(bin_op) => {
            refs.extend(collect_references_from_expression(&bin_op.left_expression));
            refs.extend(collect_references_from_expression(&bin_op.right_expression));
        }
        solc::ast::Expression::Conditional(cond) => {
            refs.extend(collect_references_from_expression(&cond.condition));
            refs.extend(collect_references_from_expression(&cond.true_expression));
            refs.extend(collect_references_from_expression(&cond.false_expression));
        }
        solc::ast::Expression::IndexAccess(idx) => {
            refs.extend(collect_references_from_expression(&idx.base_expression));
            if let Some(index) = &idx.index_expression {
                refs.extend(collect_references_from_expression(index));
            }
        }
        solc::ast::Expression::IndexRangeAccess(range) => {
            refs.extend(collect_references_from_expression(&range.base_expression));
            if let Some(start) = &range.start_expression {
                refs.extend(collect_references_from_expression(start));
            }
        }
        solc::ast::Expression::TupleExpression(tuple) => {
            for expr in tuple.components.iter().flatten() {
                refs.extend(collect_references_from_expression(expr));
            }
        }
        solc::ast::Expression::UnaryOperation(unary) => {
            refs.extend(collect_references_from_expression(&unary.sub_expression));
        }
        solc::ast::Expression::ExpressionStatement(stmt) => {
            refs.extend(collect_references_from_expression(&stmt.expression));
        }
        solc::ast::Expression::VariableDeclarationStatement(stmt) => {
            if let Some(init) = &stmt.initial_value {
                refs.extend(collect_references_from_expression(init));
            }
        }
        _ => {}
    }
    refs
}

/// Collect referenced declaration IDs from a function call.
fn collect_references_from_function_call(call: &solc::ast::FunctionCall) -> Vec<i64> {
    let mut refs = collect_references_from_function_call_expression(&call.expression);
    for arg in &call.arguments {
        refs.extend(collect_references_from_expression(arg));
    }
    refs
}

/// Collect referenced declaration IDs from a statement.
fn collect_references_from_statement(stmt: &solc::ast::Statement) -> Vec<i64> {
    let mut refs = Vec::new();
    match stmt {
        solc::ast::Statement::ExpressionStatement(expr_stmt) => {
            refs.extend(collect_references_from_expression(&expr_stmt.expression));
        }
        solc::ast::Statement::Return(ret) => {
            if let Some(expr) = &ret.expression {
                refs.extend(collect_references_from_expression(expr));
            }
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            refs.extend(collect_references_from_expression(&if_stmt.condition));
            refs.extend(collect_references_from_statement(&if_stmt.true_body));
            if let Some(false_body) = &if_stmt.false_body {
                refs.extend(collect_references_from_statement(false_body));
            }
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            if let Some(init) = &for_stmt.initialization_expression {
                refs.extend(collect_references_from_expression(init));
            }
            refs.extend(collect_references_from_expression(&for_stmt.condition));
            if let Some(loop_expr) = &for_stmt.loop_expression {
                refs.extend(collect_references_from_expression(loop_expr));
            }
            refs.extend(collect_references_from_statement(&for_stmt.body));
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            refs.extend(collect_references_from_expression(&while_stmt.condition));
            refs.extend(collect_references_from_statement(&while_stmt.body));
        }
        solc::ast::Statement::DoWhileStatement(do_while) => {
            refs.extend(collect_references_from_statement(&do_while.body));
            refs.extend(collect_references_from_expression(&do_while.condition));
        }
        solc::ast::Statement::VariableDeclarationStatement(var_stmt) => {
            if let Some(init) = &var_stmt.initial_value {
                refs.extend(collect_references_from_expression(init));
            }
        }
        solc::ast::Statement::Block(block) => {
            for stmt in &block.statements {
                refs.extend(collect_references_from_statement(stmt));
            }
        }
        solc::ast::Statement::UncheckedBlock(unchecked) => {
            for stmt in &unchecked.statements {
                refs.extend(collect_references_from_statement(stmt));
            }
        }
        solc::ast::Statement::EmitStatement(emit) => {
            refs.extend(collect_references_from_function_call(&emit.event_call));
        }
        solc::ast::Statement::TryStatement(try_stmt) => {
            refs.extend(collect_references_from_expression(&try_stmt.external_call));
            for clause in &try_stmt.clauses {
                for stmt in &clause.block.statements {
                    refs.extend(collect_references_from_statement(stmt));
                }
            }
        }
        solc::ast::Statement::RevertStatement(revert) => {
            refs.extend(collect_references_from_function_call(&revert.error_call));
        }
        _ => {}
    }
    refs
}

/// Collect all referenced declaration IDs from a function body.
fn collect_references_from_body(body: &Option<solc::ast::Block>) -> Vec<i64> {
    let mut refs = Vec::new();
    if let Some(block) = body {
        for stmt in &block.statements {
            refs.extend(collect_references_from_statement(stmt));
        }
    }
    refs
}

/// Collect all contract symbols into a map keyed by declaration ID.
fn collect_contract_symbols<'a>(
    ast: &'a solc::ast::SourceUnit,
    contract_name: &str,
) -> Option<HashMap<i64, &'a solc::ast::ContractDefinitionNode>> {
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

/// A line hit record for a single source line.
#[derive(Debug, Clone)]
pub struct LineHit {
    pub line: usize,
    pub hit_count: u64,
    pub content: String,
}

/// Kind of coverage symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Variable,
}

/// Coverage summary for a single symbol (function or variable).
///
/// Children are recursively resolved so that a target function can include
/// sub-functions, state variables, and any other symbols it transitively
/// depends on.
#[derive(Debug, Clone)]
pub struct SymbolCoverage {
    pub kind: SymbolKind,
    pub name: String,
    pub project_path: PathBuf,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub covered_lines: usize,
    pub line_hits: Vec<LineHit>,
    pub children: Vec<SymbolCoverage>,
}

/// A coverage report generated for a fuzzing campaign.
#[derive(Debug, Clone)]
pub struct CoverageReport {
    pub symbol_coverages: Vec<SymbolCoverage>,
    pub summary_total_lines: usize,
    pub summary_covered_lines: usize,
}

/// Build a [`SymbolCoverage`] for a single contract node (function or variable).
fn build_symbol_coverage(
    node: &solc::ast::ContractDefinitionNode,
    project_path: impl AsRef<Path>,
    source_index: &HashMap<usize, PathBuf>,
    artifact_path: impl AsRef<Path>,
    source_files: &HashMap<PathBuf, SourceFile>,
    line_hits: &HashMap<(PathBuf, usize), u64>,
) -> Option<SymbolCoverage> {
    let project_path = project_path.as_ref();
    let artifact_path = artifact_path.as_ref();
    let empty_source = SourceFile {
        content: String::new(),
        line_offsets: Vec::new(),
    };

    match node {
        solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
            let source_path = source_index
                .get(&func.src.source_index)
                .cloned()
                .unwrap_or_else(|| artifact_path.to_path_buf());
            let file = source_files.get(&source_path).unwrap_or(&empty_source);
            let start_line = file.offset_to_line(func.src.offset);
            let end_line = file.offset_to_line(func.src.offset + func.src.length);
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

            Some(SymbolCoverage {
                kind: SymbolKind::Function,
                name: build_signature_from_ast(func),
                project_path: project_path.to_path_buf(),
                file_path: source_path,
                start_line,
                end_line,
                total_lines,
                covered_lines,
                line_hits: hits,
                children: Vec::new(),
            })
        }
        solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
            let source_path = source_index
                .get(&var.src.source_index)
                .cloned()
                .unwrap_or_else(|| artifact_path.to_path_buf());
            let file = source_files.get(&source_path).unwrap_or(&empty_source);
            let start_line = file.offset_to_line(var.src.offset);
            let end_line = file.offset_to_line(var.src.offset + var.src.length);
            let total_lines = end_line.saturating_sub(start_line) + 1;

            let mut hits = Vec::new();
            for line in start_line..=end_line {
                let content: String = file
                    .content
                    .lines()
                    .nth(line.saturating_sub(1))
                    .unwrap_or("")
                    .into();
                hits.push(LineHit {
                    line,
                    hit_count: 0,
                    content,
                });
            }

            Some(SymbolCoverage {
                kind: SymbolKind::Variable,
                name: var.name.clone(),
                project_path: project_path.to_path_buf(),
                file_path: source_path,
                start_line,
                end_line,
                total_lines,
                covered_lines: 0,
                line_hits: hits,
                children: Vec::new(),
            })
        }
        _ => None,
    }
}

/// Recursively resolve children for a function symbol.
#[allow(clippy::too_many_arguments)]
fn resolve_children(
    coverage: &mut SymbolCoverage,
    contract_symbols: &HashMap<i64, &solc::ast::ContractDefinitionNode>,
    project_path: impl AsRef<Path>,
    source_index: &HashMap<usize, PathBuf>,
    artifact_path: impl AsRef<Path>,
    source_files: &HashMap<PathBuf, SourceFile>,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    visited: &mut HashSet<i64>,
) {
    let project_path = project_path.as_ref();
    let artifact_path = artifact_path.as_ref();
    let Some(solc::ast::ContractDefinitionNode::FunctionDefinition(func)) =
        contract_symbols.values().find(|node| match node {
            solc::ast::ContractDefinitionNode::FunctionDefinition(f) => {
                build_signature_from_ast(f) == coverage.name
            }
            _ => false,
        })
    else {
        return;
    };

    let refs = collect_references_from_body(&func.body);
    for rid in refs {
        let Some(node) = contract_symbols.get(&rid) else {
            continue;
        };
        let Some(mut child) = build_symbol_coverage(
            node,
            project_path,
            source_index,
            artifact_path,
            source_files,
            line_hits,
        ) else {
            continue;
        };
        if child.kind == SymbolKind::Function && visited.insert(rid) {
            resolve_children(
                &mut child,
                contract_symbols,
                project_path,
                source_index,
                artifact_path,
                source_files,
                line_hits,
                visited,
            );
        }
        coverage.children.push(child);
    }
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
        project_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let project_path = project_path.as_ref();
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

        let Some(contract_symbols) = collect_contract_symbols(target_artifact.ast(), contract_name)
        else {
            debug!(contract_name, "contract not found in AST");
            return Err(anyhow::anyhow!("contract not found in AST"));
        };

        // Build symbol coverage reports for target functions.
        let mut symbol_coverages: Vec<SymbolCoverage> = Vec::new();
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

            let mut symbol_coverage = SymbolCoverage {
                kind: SymbolKind::Function,
                name: func.signature(),
                project_path: project_path.to_path_buf(),
                file_path: source_path.clone(),
                start_line,
                end_line,
                total_lines,
                covered_lines,
                line_hits: hits,
                children: Vec::new(),
            };

            let mut visited = HashSet::new();
            resolve_children(
                &mut symbol_coverage,
                &contract_symbols,
                project_path,
                source_index,
                &artifact.id().path,
                source_files,
                &line_hits,
                &mut visited,
            );

            symbol_coverages.push(symbol_coverage);
        }
        debug!(
            symbol_count = symbol_coverages.len(),
            "built symbol coverages"
        );

        let summary_total_lines = symbol_coverages.iter().map(|s| s.total_lines).sum();
        let summary_covered_lines = symbol_coverages.iter().map(|s| s.covered_lines).sum();

        Ok(Self {
            symbol_coverages,
            summary_total_lines,
            summary_covered_lines,
        })
    }
}

fn write_symbol_coverage(symbol: &SymbolCoverage, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "symbol: {}", symbol.name)?;
    writeln!(
        f,
        "source: {}#L{}-L{}",
        symbol.file_path.display(),
        symbol.start_line,
        symbol.end_line
    )?;
    writeln!(f, "project: {}", symbol.project_path.display())?;
    writeln!(f)?;
    writeln!(f, "line | hits |")?;
    writeln!(f, "---- | ---- |")?;
    for hit in &symbol.line_hits {
        if hit.hit_count > 0 {
            writeln!(f, "{:4} | {:4} |{}", hit.line, hit.hit_count, hit.content)?;
        } else {
            writeln!(f, "     |      |{}", hit.content)?;
        }
    }
    writeln!(f)
}

fn write_symbol_coverage_flat(
    symbol: &SymbolCoverage,
    f: &mut fmt::Formatter<'_>,
    visited: &mut HashSet<String>,
) -> fmt::Result {
    if visited.insert(symbol.name.clone()) {
        write_symbol_coverage(symbol, f)?;
    }
    for child in &symbol.children {
        write_symbol_coverage_flat(child, f, visited)?;
    }
    Ok(())
}

impl fmt::Display for SymbolCoverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summary_uncovered = self.total_lines.saturating_sub(self.covered_lines);
        let summary_pct = if self.total_lines > 0 {
            (self.covered_lines as f64 / self.total_lines as f64) * 100.0
        } else {
            0.0
        };

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "total lines: {}", self.total_lines)?;
        writeln!(f, "total covered lines: {}", self.covered_lines)?;
        writeln!(f, "total uncovered lines: {}", summary_uncovered)?;
        writeln!(f, "coverage: {:.2}%\n", summary_pct)?;
        writeln!(f, "SOURCES\n")?;

        let mut visited = HashSet::new();
        write_symbol_coverage_flat(self, f, &mut visited)
    }
}

impl fmt::Display for CoverageReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summary_uncovered = self
            .summary_total_lines
            .saturating_sub(self.summary_covered_lines);
        let summary_pct = if self.summary_total_lines > 0 {
            (self.summary_covered_lines as f64 / self.summary_total_lines as f64) * 100.0
        } else {
            0.0
        };

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "total lines: {}", self.summary_total_lines)?;
        writeln!(f, "total covered lines: {}", self.summary_covered_lines)?;
        writeln!(f, "total uncovered lines: {}", summary_uncovered)?;
        writeln!(f, "coverage: {:.2}%\n", summary_pct)?;
        writeln!(f, "SOURCES\n")?;

        let mut visited = HashSet::new();
        for symbol in &self.symbol_coverages {
            write_symbol_coverage_flat(symbol, f, &mut visited)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use alloy_primitives::U256;
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

        interface CoverageReportInternalFunctions {
            function add_and_sub(uint256 a, uint256 b) external returns (uint256);
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

        let build_info_dir =
            std::path::Path::new("fixtures/target-contract-coverage/out/build-info");
        let mut source_index = HashMap::new();
        if let Ok(entries) = fs::read_dir(build_info_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension() != Some("json".as_ref()) {
                    continue;
                }
                let content = fs::read_to_string(&path).unwrap();
                let json: serde_json::Value = serde_json::from_str(&content).unwrap();
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
        }

        let mut source_files = HashMap::new();
        for path in source_index.values().cloned() {
            let full_path = std::path::Path::new("fixtures/target-contract-coverage").join(&path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                source_files.insert(path, super::SourceFile::new(content));
            }
        }
        let artifact_path = contract.artifact_id.path.clone();
        if !source_files.contains_key(&artifact_path) {
            let full_path =
                std::path::Path::new("fixtures/target-contract-coverage").join(&artifact_path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                source_files.insert(artifact_path, super::SourceFile::new(content));
            }
        }

        let report = super::CoverageReport::build(
            &global,
            &contract,
            &build_artifacts,
            &runtime_code,
            &source_index,
            &source_files,
            std::path::Path::new("fixtures/target-contract-coverage"),
        )
        .unwrap();

        assert!(
            !report.symbol_coverages.is_empty(),
            "coverage report should contain symbol coverages even when build artifacts include interfaces"
        );
    }

    /// Coverage report for a contract with internal functions that read and
    /// write storage must produce a valid display output.
    #[test]
    fn coverage_report_internal_functions() {
        let contract = load_coverage_fixture(
            "src/CoverageReportInternalFunctions.sol:CoverageReportInternalFunctions",
        );

        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();
        let runtime_code = deployment.result.output.unwrap_or_default();

        let global = SharedCoverage::new();
        let txs = vec![
            Transaction::new(target).calldata(Bytes::from(
                CoverageReportInternalFunctions::add_and_subCall::new((
                    U256::from(123),
                    U256::from(123),
                ))
                .abi_encode(),
            )),
        ];
        let exec = chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let build_artifacts = project.load_artifacts().unwrap();

        let build_info_dir =
            std::path::Path::new("fixtures/target-contract-coverage/out/build-info");
        let mut source_index = HashMap::new();
        if let Ok(entries) = fs::read_dir(build_info_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension() != Some("json".as_ref()) {
                    continue;
                }
                let content = fs::read_to_string(&path).unwrap();
                let json: serde_json::Value = serde_json::from_str(&content).unwrap();
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
        }

        let mut source_files = HashMap::new();
        for path in source_index.values().cloned() {
            let full_path = std::path::Path::new("fixtures/target-contract-coverage").join(&path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                source_files.insert(path, super::SourceFile::new(content));
            }
        }
        let artifact_path = contract.artifact_id.path.clone();
        if !source_files.contains_key(&artifact_path) {
            let full_path =
                std::path::Path::new("fixtures/target-contract-coverage").join(&artifact_path);
            if let Ok(content) = fs::read_to_string(&full_path) {
                source_files.insert(artifact_path, super::SourceFile::new(content));
            }
        }

        let report = super::CoverageReport::build(
            &global,
            &contract,
            &build_artifacts,
            &runtime_code,
            &source_index,
            &source_files,
            std::path::Path::new("fixtures/target-contract-coverage"),
        )
        .unwrap();

        let expected_file = "fixtures/target-contract-coverage/expected/CoverageReportInternalFunctions_add_and_sub.txt";
        let symbol_cov = report
            .symbol_coverages
            .iter()
            .find(|s| s.name == "add_and_sub(uint256,uint256)")
            .expect("add_and_sub coverage must be present");
        let formatted = format!("{symbol_cov}");
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }
}
