//! Coverage reporter for building human-readable coverage reports.
//!
//! [`CoverageReporter`] is the presentation layer of the coverage reporting
//! pipeline. It takes three inputs:
//!
//! 1. [`SharedCoverage`] - the global, merged coverage map containing raw PC
//!    hit counts from every fuzzer execution during a campaign.
//! 2. **Target functions** - the ABI functions the fuzzer exercised, which the
//!    reporter will produce individual reports for.
//! 3. [`CoverageContext`] - the data layer that maps bytecode hits back to
//!    source lines and AST nodes.
//!
//! The reporter uses the context to:
//!
//! - Call [`CoverageContext::build_line_hits`] and turn raw PC hits into a
//!   `(source_path, line) -> hit_count` map.
//! - Look up the AST function definition that matches each target function
//!   signature.
//! - Walk the function body to discover referenced internal functions and state
//!   variables, building [`SourceCoverage`] entries for each symbol.
//! - Recursively resolve child references so that internal helpers and
//!   modifiers are included in the report.
//!
//! The output structs are:
//!
//! - [`FunctionReport`] - a detailed, per-function breakdown with line-by-line
//!   hits and coverage percentage.
//! - [`SourceCoverage`] - coverage for a single source symbol (function or
//!   variable) within a function report.
//!
//! [`FunctionReport`] implements `Display` for console-friendly formatting.
//!
//! # Executable vs non-executable lines
//!
//! A line is **executable** when the compiler's source map maps at least one
//! program counter to it. A line is **non-executable** when no PC maps to it.
//!
//! Examples of non-executable lines:
//!
//! - Storage variable definitions (`uint256 public x;`)
//! - Closing braces (`}`)
//! - Empty lines and comments
//!
//! The reporter marks each line with `is_executable` via [`LineHits`]. In the
//! `Display` output:
//!
//! - An executable line that was hit shows the exact hit count (e.g. `1`, `2`).
//! - An executable line that was not hit shows `0`.
//! - A non-executable line shows an empty hits column.
//!
//! In short: `CoverageContext` knows what code was hit and where it lives in
//! source; `CoverageReporter` decides how to present that information.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};

use alloy_json_abi::Function;
use rayon::prelude::*;

use crate::evm::coverage::context::{CoverageContext, SymbolTable};
use crate::evm::coverage::shared::SharedCoverage;
use crate::formatter;

// ----------------------------------------------------------------------------
// Report data types
// ----------------------------------------------------------------------------

/// A single line hit record.
#[derive(Debug, Clone)]
pub struct LineHits {
    pub line: usize,
    pub hit_count: u64,
    pub is_executable: bool,
    pub content: String,
}

/// Coverage for a single source symbol (function or variable).
#[derive(Debug, Clone)]
pub struct SourceCoverage {
    pub symbol: String,
    pub source: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub line_hits: Vec<LineHits>,
    pub project: PathBuf,
}

/// A detailed coverage report for a single target function.
#[derive(Debug, Clone)]
pub struct FunctionReport {
    pub executable_line_count: usize,
    pub non_executable_line_count: usize,
    pub executable_line_covered: usize,
    pub coverage: f64,
    pub source_coverages: Vec<SourceCoverage>,
}

/// Orchestrates the building of coverage reports.
#[derive(Debug, Clone)]
pub struct CoverageReporter {
    coverage: SharedCoverage,
    target_functions: Vec<Function>,
    context: CoverageContext,
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
            coverage: SharedCoverage::new(),
            target_functions: Vec::new(),
            context: CoverageContext::default(),
        }
    }

    /// Set the [`SharedCoverage`] data.
    pub fn coverage(mut self, coverage: SharedCoverage) -> Self {
        self.coverage = coverage;
        self
    }

    /// Set the target functions to report on.
    pub fn target_functions(mut self, target_functions: Vec<Function>) -> Self {
        self.target_functions = target_functions;
        self
    }

    /// Set the [`CoverageContext`] for lookups.
    pub fn context(mut self, context: CoverageContext) -> Self {
        self.context = context;
        self
    }

    /// Return detailed reports for all target functions.
    ///
    /// This method computes the shared line-hit and executable-line maps once,
    /// then builds each per-function report in parallel with `rayon`.
    pub fn get_reports(&self) -> Vec<(String, FunctionReport)> {
        let line_hits = self.context.build_line_hits(&self.coverage);
        let executable_lines = self.context.build_executable_lines();
        let Some(artifact) = self.context.target_artifact() else {
            return Vec::new();
        };
        let contract_name = artifact.name();
        let Some(symbols) = self
            .context
            .resolve_contract_symbols(artifact, contract_name)
        else {
            return Vec::new();
        };

        self.target_functions
            .par_iter()
            .filter_map(|func| {
                let signature = func.signature();
                let func_def =
                    self.context
                        .resolve_function_definition(artifact, contract_name, func)?;
                let report = build_function_report(
                    func_def,
                    &self.context,
                    &line_hits,
                    &executable_lines,
                    &symbols,
                    &signature,
                )?;
                Some((signature, report))
            })
            .collect()
    }
}

// ----------------------------------------------------------------------------
// Display
// ----------------------------------------------------------------------------

impl fmt::Display for FunctionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pct = self.coverage;
        let total_line_count = self.executable_line_count + self.non_executable_line_count;

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "coverage: {:.2}%", pct)?;
        writeln!(f, "line count:")?;
        writeln!(f, "  total: {}", total_line_count)?;
        writeln!(f, "  executable: {}", self.executable_line_count)?;
        writeln!(f, "  non executable: {}", self.non_executable_line_count)?;
        writeln!(f, "  covered: {}", self.executable_line_covered)?;
        writeln!(f)?;
        writeln!(f, "SOURCES\n")?;

        for source in &self.source_coverages {
            writeln!(f, "symbol: {}", source.symbol)?;
            writeln!(
                f,
                "source: {}#L{}-L{}",
                source.source.display(),
                source.start_line,
                source.end_line
            )?;
            writeln!(f, "project: {}", source.project.display())?;
            writeln!(f)?;
            writeln!(f, "line |   hits |")?;
            writeln!(f, "---- | ------ |")?;
            for hit in &source.line_hits {
                if hit.is_executable && hit.hit_count > 0 {
                    writeln!(
                        f,
                        "{:4} | {:>6} |{}",
                        hit.line,
                        formatter::kmb(hit.hit_count),
                        hit.content
                    )?;
                } else if hit.is_executable {
                    writeln!(f, "{:4} | {:>6} |{}", hit.line, 0, hit.content)?;
                } else {
                    writeln!(f, "{:4} | {:>6} |{}", hit.line, "", hit.content)?;
                }
            }
            writeln!(f)?;
        }

        Ok(())
    }
}

// ----------------------------------------------------------------------------
// Report builders
// ----------------------------------------------------------------------------

fn build_function_report(
    func_def: &solc::ast::FunctionDefinition,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    executable_lines: &HashSet<(PathBuf, usize)>,
    symbols: &SymbolTable,
    function_signature: &str,
) -> Option<FunctionReport> {
    let source_path = match context.resolve_source_index(func_def.src.source_index) {
        Some(p) => p.to_path_buf(),
        None => context
            .target_artifact()
            .map(|a| a.id().path.to_path_buf())
            .unwrap_or_default(),
    };

    let file = context.resolve_source_file(&source_path);
    let start_line = file
        .map(|f| f.offset_to_line(func_def.src.offset))
        .unwrap_or(1);
    let end_line = file
        .map(|f| f.offset_to_line(func_def.src.offset + func_def.src.length))
        .unwrap_or(start_line);

    let mut executable_line_count = 0;
    let mut non_executable_line_count = 0;
    let mut executable_line_covered = 0;
    let mut hits = Vec::new();
    for line in start_line..=end_line {
        let is_executable = executable_lines.contains(&(source_path.clone(), line));
        let hit_count = line_hits
            .get(&(source_path.clone(), line))
            .copied()
            .unwrap_or(0);
        if is_executable {
            executable_line_count += 1;
            if hit_count > 0 {
                executable_line_covered += 1;
            }
        } else {
            non_executable_line_count += 1;
        }
        let content = file
            .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
            .unwrap_or("")
            .into();
        hits.push(LineHits {
            line,
            hit_count,
            is_executable,
            content,
        });
    }

    let project = context
        .target_artifact()
        .map(|a| a.project_path().to_path_buf())
        .unwrap_or_default();

    let source_coverage = SourceCoverage {
        symbol: function_signature.into(),
        source: source_path.clone(),
        start_line,
        end_line,
        line_hits: hits,
        project: project.to_path_buf(),
    };

    let mut source_coverages = vec![source_coverage];
    let mut total_executable_line_count = executable_line_count;
    let mut total_non_executable_line_count = non_executable_line_count;
    let mut total_executable_line_covered = executable_line_covered;
    let mut visited_ids = HashSet::new();
    visited_ids.insert(func_def.id);

    let child_result = resolve_children(
        func_def,
        context,
        line_hits,
        executable_lines,
        symbols,
        &mut visited_ids,
        &project,
    );

    total_executable_line_count += child_result.executable_line_count;
    total_non_executable_line_count += child_result.non_executable_line_count;
    total_executable_line_covered += child_result.executable_line_covered;
    source_coverages.extend(child_result.children);

    let total_coverage = if total_executable_line_count > 0 {
        (total_executable_line_covered as f64 / total_executable_line_count as f64) * 100.0
    } else {
        0.0
    };

    Some(FunctionReport {
        executable_line_count: total_executable_line_count,
        non_executable_line_count: total_non_executable_line_count,
        executable_line_covered: total_executable_line_covered,
        coverage: total_coverage,
        source_coverages,
    })
}

/// Coverage for a source symbol together with its line counts.
struct SourceCoverageWithCounts {
    coverage: SourceCoverage,
    executable_line_count: usize,
    non_executable_line_count: usize,
    executable_line_covered: usize,
}

fn build_source_coverage(
    node: &solc::ast::ContractDefinitionNode,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    executable_lines: &HashSet<(PathBuf, usize)>,
    project: &Path,
) -> Option<SourceCoverageWithCounts> {
    match node {
        solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
            let source_path = context
                .resolve_source_index(func.src.source_index)
                .cloned()
                .unwrap_or_default();
            let file = context.resolve_source_file(&source_path);
            let start_line = file.map(|f| f.offset_to_line(func.src.offset)).unwrap_or(1);
            let end_line = file
                .map(|f| f.offset_to_line(func.src.offset + func.src.length))
                .unwrap_or(start_line);

            let mut executable = 0;
            let mut non_executable = 0;
            let mut covered = 0;
            let mut hits = Vec::new();
            for line in start_line..=end_line {
                let is_executable = executable_lines.contains(&(source_path.clone(), line));
                let hit_count = line_hits
                    .get(&(source_path.clone(), line))
                    .copied()
                    .unwrap_or(0);
                if is_executable {
                    executable += 1;
                    if hit_count > 0 {
                        covered += 1;
                    }
                } else {
                    non_executable += 1;
                }
                let content = file
                    .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
                    .unwrap_or("")
                    .into();
                hits.push(LineHits {
                    line,
                    hit_count,
                    is_executable,
                    content,
                });
            }

            Some(SourceCoverageWithCounts {
                coverage: SourceCoverage {
                    symbol: build_signature_from_ast(func),
                    source: source_path,
                    start_line,
                    end_line,
                    line_hits: hits,
                    project: project.to_path_buf(),
                },
                executable_line_count: executable,
                non_executable_line_count: non_executable,
                executable_line_covered: covered,
            })
        }
        solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
            let source_path = context
                .resolve_source_index(var.src.source_index)
                .cloned()
                .unwrap_or_default();
            let file = context.resolve_source_file(&source_path);
            let start_line = file.map(|f| f.offset_to_line(var.src.offset)).unwrap_or(1);
            let end_line = file
                .map(|f| f.offset_to_line(var.src.offset + var.src.length))
                .unwrap_or(start_line);

            let mut executable = 0;
            let mut non_executable = 0;
            let mut covered = 0;
            let mut hits = Vec::new();
            // State variable definitions are non-executable.
            let is_state_variable = var.state_variable;
            for line in start_line..=end_line {
                let is_executable =
                    !is_state_variable && executable_lines.contains(&(source_path.clone(), line));
                let hit_count = line_hits
                    .get(&(source_path.clone(), line))
                    .copied()
                    .unwrap_or(0);
                if is_executable {
                    executable += 1;
                    if hit_count > 0 {
                        covered += 1;
                    }
                } else {
                    non_executable += 1;
                }
                let content = file
                    .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
                    .unwrap_or("")
                    .into();
                hits.push(LineHits {
                    line,
                    hit_count,
                    is_executable,
                    content,
                });
            }

            Some(SourceCoverageWithCounts {
                coverage: SourceCoverage {
                    symbol: var.name.clone(),
                    source: source_path,
                    start_line,
                    end_line,
                    line_hits: hits,
                    project: project.to_path_buf(),
                },
                executable_line_count: executable,
                non_executable_line_count: non_executable,
                executable_line_covered: covered,
            })
        }
        _ => None,
    }
}

struct ResolveChildrenResult {
    children: Vec<SourceCoverage>,
    executable_line_count: usize,
    non_executable_line_count: usize,
    executable_line_covered: usize,
}

impl ResolveChildrenResult {
    fn merge_child(&mut self, other: ResolveChildrenResult) {
        self.children.extend(other.children);
        self.executable_line_count += other.executable_line_count;
        self.non_executable_line_count += other.non_executable_line_count;
        self.executable_line_covered += other.executable_line_covered;
    }
}

fn resolve_children(
    func_def: &solc::ast::FunctionDefinition,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    executable_lines: &HashSet<(PathBuf, usize)>,
    symbols: &SymbolTable,
    visited_ids: &mut HashSet<i64>,
    project: &Path,
) -> ResolveChildrenResult {
    let mut result = ResolveChildrenResult {
        children: Vec::new(),
        executable_line_count: 0,
        non_executable_line_count: 0,
        executable_line_covered: 0,
    };

    let refs = collect_references_from_body(&func_def.body);
    let ref_names = collect_reference_names(&func_def.body);
    for rid in refs {
        let node = if let Some(n) = symbols.get_by_id(rid) {
            Some(n)
        } else if let Some(name) = ref_names.get(&rid) {
            symbols.get_by_name(name)
        } else {
            None
        };
        let Some(node) = node else {
            continue;
        };

        // If the reference is an unimplemented interface function, try to
        // find the actual implementation by matching the function selector.
        // We prefer implementations that have line hits in the coverage map.
        let mut nodes_to_add: Vec<(&solc::ast::ContractDefinitionNode, i64)> = Vec::new();
        let mut impl_resolved = false;
        if let solc::ast::ContractDefinitionNode::FunctionDefinition(func) = node
            && !func.implemented
            && let Some(ref selector) = func.function_selector
        {
            let impls = context.find_implementations_by_selector(selector);
            for impl_node in impls {
                let impl_id = match impl_node {
                    solc::ast::ContractDefinitionNode::FunctionDefinition(f) => f.id,
                    solc::ast::ContractDefinitionNode::VariableDeclaration(v) => v.id,
                    _ => continue,
                };
                if visited_ids.contains(&impl_id) {
                    impl_resolved = true;
                    continue;
                }
                let Some(child) =
                    build_source_coverage(impl_node, context, line_hits, executable_lines, project)
                else {
                    continue;
                };
                if child.coverage.line_hits.iter().any(|h| h.hit_count > 0) {
                    nodes_to_add.push((impl_node, impl_id));
                    impl_resolved = true;
                }
            }
        }

        // Fall back to the original node if no implementation was found.
        if nodes_to_add.is_empty() && !impl_resolved {
            nodes_to_add.push((node, rid));
        }

        for (node_to_add, id_to_add) in nodes_to_add {
            if !visited_ids.insert(id_to_add) {
                continue;
            }

            let Some(child) =
                build_source_coverage(node_to_add, context, line_hits, executable_lines, project)
            else {
                continue;
            };

            result.executable_line_count += child.executable_line_count;
            result.non_executable_line_count += child.non_executable_line_count;
            result.executable_line_covered += child.executable_line_covered;
            result.children.push(child.coverage);

            if let solc::ast::ContractDefinitionNode::FunctionDefinition(f) = node_to_add {
                let sub = resolve_children(
                    f,
                    context,
                    line_hits,
                    executable_lines,
                    symbols,
                    visited_ids,
                    project,
                );
                result.merge_child(sub);
            }
        }
    }

    result
}

// ----------------------------------------------------------------------------
// AST helpers
// ----------------------------------------------------------------------------

/// Build a Solidity signature from an AST function definition.
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

fn collect_references_from_function_call(call: &solc::ast::FunctionCall) -> Vec<i64> {
    let mut refs = collect_references_from_function_call_expression(&call.expression);
    for arg in &call.arguments {
        refs.extend(collect_references_from_expression(arg));
    }
    refs
}

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

fn collect_references_from_body(body: &Option<solc::ast::Block>) -> Vec<i64> {
    let mut refs = Vec::new();
    if let Some(block) = body {
        for stmt in &block.statements {
            refs.extend(collect_references_from_statement(stmt));
        }
    }
    refs
}

/// Collect a map from reference id to the referenced symbol's name.
fn collect_reference_names(body: &Option<solc::ast::Block>) -> HashMap<i64, String> {
    let mut names = HashMap::new();
    if let Some(block) = body {
        for stmt in &block.statements {
            collect_reference_names_from_statement(stmt, &mut names);
        }
    }
    names
}

fn collect_reference_names_from_statement(
    stmt: &solc::ast::Statement,
    names: &mut HashMap<i64, String>,
) {
    match stmt {
        solc::ast::Statement::ExpressionStatement(expr_stmt) => {
            collect_reference_names_from_expression(&expr_stmt.expression, names);
        }
        solc::ast::Statement::Return(ret) => {
            if let Some(expr) = &ret.expression {
                collect_reference_names_from_expression(expr, names);
            }
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            collect_reference_names_from_expression(&if_stmt.condition, names);
            collect_reference_names_from_statement(&if_stmt.true_body, names);
            if let Some(false_body) = &if_stmt.false_body {
                collect_reference_names_from_statement(false_body, names);
            }
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            if let Some(init) = &for_stmt.initialization_expression {
                collect_reference_names_from_expression(init, names);
            }
            collect_reference_names_from_expression(&for_stmt.condition, names);
            if let Some(loop_expr) = &for_stmt.loop_expression {
                collect_reference_names_from_expression(loop_expr, names);
            }
            collect_reference_names_from_statement(&for_stmt.body, names);
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            collect_reference_names_from_expression(&while_stmt.condition, names);
            collect_reference_names_from_statement(&while_stmt.body, names);
        }
        solc::ast::Statement::DoWhileStatement(do_while) => {
            collect_reference_names_from_statement(&do_while.body, names);
            collect_reference_names_from_expression(&do_while.condition, names);
        }
        solc::ast::Statement::VariableDeclarationStatement(var_stmt) => {
            if let Some(init) = &var_stmt.initial_value {
                collect_reference_names_from_expression(init, names);
            }
        }
        solc::ast::Statement::Block(block) => {
            for stmt in &block.statements {
                collect_reference_names_from_statement(stmt, names);
            }
        }
        solc::ast::Statement::UncheckedBlock(unchecked) => {
            for stmt in &unchecked.statements {
                collect_reference_names_from_statement(stmt, names);
            }
        }
        solc::ast::Statement::EmitStatement(emit) => {
            collect_reference_names_from_function_call(&emit.event_call, names);
        }
        solc::ast::Statement::TryStatement(try_stmt) => {
            collect_reference_names_from_expression(&try_stmt.external_call, names);
            for clause in &try_stmt.clauses {
                for stmt in &clause.block.statements {
                    collect_reference_names_from_statement(stmt, names);
                }
            }
        }
        solc::ast::Statement::RevertStatement(revert) => {
            collect_reference_names_from_function_call(&revert.error_call, names);
        }
        _ => {}
    }
}

fn collect_reference_names_from_expression(
    expr: &solc::ast::Expression,
    names: &mut HashMap<i64, String>,
) {
    match expr {
        solc::ast::Expression::Identifier(id) => {
            if let Some(rid) = id.referenced_declaration {
                names.insert(rid, id.name.clone());
            }
        }
        solc::ast::Expression::MemberAccess(member) => {
            if let Some(rid) = member.referenced_declaration {
                names.insert(rid, member.member_name.clone());
            }
            collect_reference_names_from_expression(&member.expression, names);
        }
        solc::ast::Expression::FunctionCall(call) => {
            collect_reference_names_from_function_call(call, names);
        }
        solc::ast::Expression::Assignment(assignment) => {
            collect_reference_names_from_expression(&assignment.left_hand_side, names);
            collect_reference_names_from_expression(&assignment.right_hand_side, names);
        }
        solc::ast::Expression::BinaryOperation(bin_op) => {
            collect_reference_names_from_expression(&bin_op.left_expression, names);
            collect_reference_names_from_expression(&bin_op.right_expression, names);
        }
        solc::ast::Expression::Conditional(cond) => {
            collect_reference_names_from_expression(&cond.condition, names);
            collect_reference_names_from_expression(&cond.true_expression, names);
            collect_reference_names_from_expression(&cond.false_expression, names);
        }
        solc::ast::Expression::IndexAccess(idx) => {
            collect_reference_names_from_expression(&idx.base_expression, names);
            if let Some(index) = &idx.index_expression {
                collect_reference_names_from_expression(index, names);
            }
        }
        solc::ast::Expression::IndexRangeAccess(range) => {
            collect_reference_names_from_expression(&range.base_expression, names);
            if let Some(start) = &range.start_expression {
                collect_reference_names_from_expression(start, names);
            }
        }
        solc::ast::Expression::TupleExpression(tuple) => {
            for expr in tuple.components.iter().flatten() {
                collect_reference_names_from_expression(expr, names);
            }
        }
        solc::ast::Expression::UnaryOperation(unary) => {
            collect_reference_names_from_expression(&unary.sub_expression, names);
        }
        solc::ast::Expression::ExpressionStatement(stmt) => {
            collect_reference_names_from_expression(&stmt.expression, names);
        }
        solc::ast::Expression::VariableDeclarationStatement(stmt) => {
            if let Some(init) = &stmt.initial_value {
                collect_reference_names_from_expression(init, names);
            }
        }
        _ => {}
    }
}

fn collect_reference_names_from_function_call(
    call: &solc::ast::FunctionCall,
    names: &mut HashMap<i64, String>,
) {
    collect_reference_names_from_function_call_expression(&call.expression, names);
    for arg in &call.arguments {
        collect_reference_names_from_expression(arg, names);
    }
}

fn collect_reference_names_from_function_call_expression(
    expr: &solc::ast::FunctionCallExpression,
    names: &mut HashMap<i64, String>,
) {
    match expr {
        solc::ast::FunctionCallExpression::Identifier(id) => {
            if let Some(rid) = id.referenced_declaration {
                names.insert(rid, id.name.clone());
            }
        }
        solc::ast::FunctionCallExpression::MemberAccess(member) => {
            if let Some(rid) = member.referenced_declaration {
                names.insert(rid, member.member_name.clone());
            }
        }
        solc::ast::FunctionCallExpression::FunctionCall(call) => {
            collect_reference_names_from_function_call(call, names);
        }
        solc::ast::FunctionCallExpression::FunctionCallOptions(options) => {
            collect_reference_names_from_expression(&options.expression, names);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::coverage::SharedCoverage;
    use crate::foundry;
    use crate::foundry::Artifact;

    use super::*;

    fn get_report(reporter: &CoverageReporter, signature: &str) -> Option<FunctionReport> {
        reporter
            .get_reports()
            .into_iter()
            .find(|(sig, _)| sig == signature)
            .map(|(_, report)| report)
    }

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }

        interface TargetContract {
            function addAndSub(uint256 a, uint256 b) external returns (uint256);
            function earlyReturn(uint256 a) external returns (uint256);
            function inheritanceCall(uint256 a) external returns (uint256);
            function libCall(uint256 amount) external returns (uint256);
            function libLinkedCall(uint256 amount) external returns (uint256);
            function interfaceCall(uint256 amount) external returns (uint256);
            function counterLinked() external returns (address);
        }

        interface EmptyTargetFunction {
            function dummyTargetFunction() external;
        }

        interface InheritedTarget {
            function inheritedTargetFunction() external;
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    struct Deployed {
        chain: Chain,
        address: Address,
    }

    fn deploy_and_setup(contract: &Contract) -> Deployed {
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let mut deploy_opts = DeployInput::new(&contract.initcode);
        for lib in &contract.libraries {
            deploy_opts = deploy_opts.add_library(lib.clone());
        }
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
        }

        Deployed {
            chain,
            address: target,
        }
    }

    /// Coverage report logic must succeed even when the build artifacts contain
    /// interface/abstract contracts that have no deployed bytecode.
    #[test]
    fn coverage_report_build_with_interface_artifact() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        assert!(
            get_report(&reporter, "branch(bool)").is_some(),
            "coverage report should contain reports even when build artifacts include interfaces"
        );
    }

    /// Coverage report for a single execution of a target function.
    #[test]
    fn coverage_report_executed_once() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::addAndSubCall::new((U256::from(123), U256::from(123))).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let expected_file = "fixtures/target-contract-coverage/expected/addAndSub.txt";
        let report = get_report(&reporter, "addAndSub(uint256,uint256)")
            .expect("addAndSub report must be present");
        let formatted = format!("{report}");
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for single execution must match expected"
        );
    }

    /// Coverage report hit counts must scale correctly when a function is
    /// executed multiple times.
    #[test]
    fn coverage_report_hit_counts_two_executions() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::addAndSubCall::new((U256::from(123), U256::from(123))).abi_encode(),
        ))];

        // Execute the same transaction twice and merge both into global coverage.
        let exec1 = deployed.chain.exec(&txs).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let exec2 = deployed.chain.exec(&txs).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        global.merge(&coverage2);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let expected_file = "fixtures/target-contract-coverage/expected/addAndSub2.txt";
        let report = get_report(&reporter, "addAndSub(uint256,uint256)")
            .expect("addAndSub report must be present");
        let formatted = format!("{report}");
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for 2x execution must match expected"
        );
    }

    /// Coverage report for a function with an early return path.
    #[test]
    fn coverage_report_early_return() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::earlyReturnCall::new((U256::from(0),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let expected_file = "fixtures/target-contract-coverage/expected/earlyReturn.txt";
        let report = get_report(&reporter, "earlyReturn(uint256)")
            .expect("earlyReturn report must be present");
        let formatted = format!("{report}");
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for early return must match expected"
        );
    }

    #[test]
    fn coverage_report_inheritance() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::inheritanceCallCall::new((U256::from(5),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report = get_report(&reporter, "inheritanceCall(uint256)")
            .expect("inheritanceCall report must be present");
        let formatted = format!("{report}");
        let expected_file = "fixtures/target-contract-coverage/expected/inheritanceCall.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for inheritance call must match expected"
        );
    }

    /// Coverage report for a function that calls a contract deployed in the
    /// constructor.
    #[test]
    fn coverage_report_lib_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::libCallCall::new((U256::from(42),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report =
            get_report(&reporter, "libCall(uint256)").expect("libCall report must be present");
        let formatted = format!("{report}");
        let expected_file = "fixtures/target-contract-coverage/expected/libCall.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for libCall must match expected"
        );
    }

    /// Coverage report for a function that calls a contract deployed in the
    /// constructor, where the called contract depends on a linked library.
    #[test]
    fn coverage_report_lib_linked_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::libLinkedCallCall::new((U256::from(42),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report = get_report(&reporter, "libLinkedCall(uint256)")
            .expect("libLinkedCall report must be present");
        let formatted = format!("{report}");
        let expected_file = "fixtures/target-contract-coverage/expected/libLinkedCall.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for libLinkedCall must match expected"
        );
    }

    /// Coverage report for a function that interacts with a contract deployed in
    /// the constructor via an interface instead of a direct call.
    #[test]
    fn coverage_report_interface_call() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::interfaceCallCall::new((U256::from(42),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report = get_report(&reporter, "interfaceCall(uint256)")
            .expect("interfaceCall report must be present");
        let formatted = format!("{report}");
        let expected_file = "fixtures/target-contract-coverage/expected/interfaceCall.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for interfaceCall must match expected"
        );
    }

    /// Regression test: coverage report must be generated for a target function
    /// with an empty body.
    #[test]
    fn coverage_report_empty_target_function() {
        let contract = load_coverage_fixture("src/EmptyTargetFunction.sol:EmptyTargetFunction");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            EmptyTargetFunction::dummyTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        assert!(
            get_report(&reporter, "dummyTargetFunction()").is_some(),
            "coverage report must be generated for an empty target function"
        );
    }

    /// Regression test: coverage report must be generated for a target function
    /// inherited from a base contract.
    #[test]
    fn coverage_report_inherited_target_function() {
        let contract = load_coverage_fixture("src/InheritedTarget.sol:InheritedTarget");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            InheritedTarget::inheritedTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = CoverageContext::from_project(&project)
            .unwrap()
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        assert!(
            get_report(&reporter, "inheritedTargetFunction()").is_some(),
            "coverage report must be generated for a target function inherited from a base contract"
        );
    }

    /// Regression test: coverage report must be generated even when a base
    /// contract referenced in `linearized_base_contracts` has a different AST id
    /// in the loaded artifact because it was compiled in a separate compilation
    /// unit.
    #[test]
    fn coverage_report_missing_base_contract_id() {
        let contract = load_coverage_fixture("src/EmptyTargetFunction.sol:EmptyTargetFunction");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            EmptyTargetFunction::dummyTargetFunctionCall::new(()).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");

        // Load the original artifact and modify it so the base contract id
        // no longer matches the loaded RaptorFuzz artifact (simulating the
        // multi-compilation-unit scenario).
        let artifact_path = "fixtures/target-contract-coverage/out/EmptyTargetFunction.sol/EmptyTargetFunction.json";
        let content = fs::read_to_string(artifact_path).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&content).unwrap();
        for node in json["ast"]["nodes"].as_array_mut().unwrap() {
            if node["nodeType"] == "ContractDefinition" {
                node["linearizedBaseContracts"] = serde_json::json!([212, 99999]);
                for bc in node["baseContracts"].as_array_mut().unwrap() {
                    bc["baseName"]["referencedDeclaration"] = serde_json::json!(99999);
                }
            }
        }
        let mut modified = Artifact::from_json_str(&serde_json::to_string(&json).unwrap()).unwrap();
        modified.set_project_path(&project.path);

        let context = CoverageContext::from_project(&project)
            .unwrap()
            .remove_artifact(&contract.artifact_id)
            .add_artifact(contract.artifact_id.clone(), modified)
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report = get_report(&reporter, "dummyTargetFunction()")
            .expect("coverage report must be generated");
        let formatted = format!("{report}");
        let expected_file = "fixtures/target-contract-coverage/expected/missingBaseContractId.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for missing base contract id must match expected"
        );
    }

    /// Regression test: coverage report must correctly resolve base contract
    /// functions by name when the base contract is from a different compilation
    /// unit and the target function calls those functions.
    #[test]
    fn coverage_report_missing_base_contract_id_with_calls() {
        let contract = load_coverage_fixture("src/TargetContract.sol:TargetContract");
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![Transaction::new(deployed.address).calldata(Bytes::from(
            TargetContract::inheritanceCallCall::new((U256::from(5),)).abi_encode(),
        ))];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");

        // Load the original artifact and modify it so the base contract id
        // no longer matches the loaded RaptorFuzz artifact (simulating the
        // multi-compilation-unit scenario).
        let artifact_path =
            "fixtures/target-contract-coverage/out/TargetContract.sol/TargetContract.json";
        let content = fs::read_to_string(artifact_path).unwrap();
        let mut json: serde_json::Value = serde_json::from_str(&content).unwrap();
        for node in json["ast"]["nodes"].as_array_mut().unwrap() {
            if node["nodeType"] == "ContractDefinition" {
                node["linearizedBaseContracts"] = serde_json::json!([650, 99999]);
                for bc in node["baseContracts"].as_array_mut().unwrap() {
                    bc["baseName"]["referencedDeclaration"] = serde_json::json!(99999);
                }
            }
        }
        let mut modified = Artifact::from_json_str(&serde_json::to_string(&json).unwrap()).unwrap();
        modified.set_project_path(&project.path);

        let context = CoverageContext::from_project(&project)
            .unwrap()
            .remove_artifact(&contract.artifact_id)
            .add_artifact(contract.artifact_id.clone(), modified)
            .with_target_artifact(&contract.artifact_id)
            .unwrap();

        let project_path = context
            .target_artifact()
            .unwrap()
            .project_path()
            .to_string_lossy()
            .to_string();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let report = get_report(&reporter, "inheritanceCall(uint256)")
            .expect("coverage report must be generated");
        let formatted = format!("{report}");
        let expected_file =
            "fixtures/target-contract-coverage/expected/missingBaseContractIdWithCalls.txt";
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        let expected = expected.replace("fixtures/target-contract-coverage", &project_path);
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output for missing base contract id with calls must match expected"
        );
    }
}
