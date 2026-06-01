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
//! - [`SummaryReport`] - campaign-level totals across all target functions.
//! - [`FunctionReport`] - a detailed, per-function breakdown with line-by-line
//!   hits and coverage percentage.
//! - [`SourceCoverage`] - coverage for a single source symbol (function or
//!   variable) within a function report.
//!
//! Both `SummaryReport` and `FunctionReport` implement `Display` for
//! console-friendly formatting.
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

use crate::evm::coverage::context::CoverageContext;
use crate::evm::coverage::shared::SharedCoverage;

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

/// A summary report for a single target function.
#[derive(Debug, Clone)]
pub struct FunctionSummaryReport {
    pub executable_line_count: usize,
    pub non_executable_line_count: usize,
    pub executable_line_covered: usize,
    pub coverage: f64,
    pub path: PathBuf,
    pub name: String,
    pub total_sub_functions: usize,
}

/// A detailed coverage report for a single target function.
#[derive(Debug, Clone)]
pub struct FunctionReport {
    pub executable_line_count: usize,
    pub non_executable_line_count: usize,
    pub executable_line_covered: usize,
    pub coverage: f64,
    pub path: PathBuf,
    pub name: String,
    pub total_sub_functions: usize,
    pub source_coverages: Vec<SourceCoverage>,
}

/// A summary report for the entire campaign.
#[derive(Debug, Clone)]
pub struct SummaryReport {
    pub total_executable_line_count: usize,
    pub total_non_executable_line_count: usize,
    pub total_executable_line_covered: usize,
    pub total_coverage: f64,
    pub function_summaries: Vec<FunctionSummaryReport>,
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

    /// Return a campaign-level summary.
    pub fn summary(&self) -> SummaryReport {
        let reports = self.reports();
        let total_executable_line_count = reports.iter().map(|r| r.executable_line_count).sum();
        let total_non_executable_line_count =
            reports.iter().map(|r| r.non_executable_line_count).sum();
        let total_executable_line_covered = reports.iter().map(|r| r.executable_line_covered).sum();
        let total_coverage = if total_executable_line_count > 0 {
            (total_executable_line_covered as f64 / total_executable_line_count as f64) * 100.0
        } else {
            0.0
        };

        let function_summaries = reports
            .into_iter()
            .map(|r| FunctionSummaryReport {
                executable_line_count: r.executable_line_count,
                non_executable_line_count: r.non_executable_line_count,
                executable_line_covered: r.executable_line_covered,
                coverage: r.coverage,
                path: r.path,
                name: r.name,
                total_sub_functions: r.total_sub_functions,
            })
            .collect();

        SummaryReport {
            total_executable_line_count,
            total_non_executable_line_count,
            total_executable_line_covered,
            total_coverage,
            function_summaries,
        }
    }

    /// Return the detailed report for a specific target function signature.
    pub fn get_report(&self, function_signature: &str) -> Option<FunctionReport> {
        let line_hits = self.context.build_line_hits(&self.coverage);
        let executable_lines = self.context.build_executable_lines();
        let artifact = self.context.target_artifact()?;
        let contract_name = artifact.name();
        let symbols = self
            .context
            .resolve_contract_symbols(artifact, contract_name)?;

        let func = self
            .target_functions
            .iter()
            .find(|f| f.signature() == function_signature)?;
        let func_def = self
            .context
            .resolve_function_definition(artifact, contract_name, func)?;

        build_function_report(
            func_def,
            &self.context,
            &line_hits,
            &executable_lines,
            &symbols,
            function_signature,
        )
    }

    /// Return detailed reports for all target functions.
    pub fn reports(&self) -> Vec<FunctionReport> {
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

        let mut reports = Vec::new();
        for func in &self.target_functions {
            let sig = func.signature();
            let Some(func_def) =
                self.context
                    .resolve_function_definition(artifact, contract_name, func)
            else {
                continue;
            };
            if let Some(report) = build_function_report(
                func_def,
                &self.context,
                &line_hits,
                &executable_lines,
                &symbols,
                &sig,
            ) {
                reports.push(report);
            }
        }
        reports
    }
}

// ----------------------------------------------------------------------------
// Display
// ----------------------------------------------------------------------------

impl fmt::Display for SummaryReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pct = self.total_coverage;
        let total_line_count =
            self.total_executable_line_count + self.total_non_executable_line_count;

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "coverage: {:.2}%", pct)?;
        writeln!(f, "line count:")?;
        writeln!(f, "  total: {}", total_line_count)?;
        writeln!(f, "  executable: {}", self.total_executable_line_count)?;
        writeln!(
            f,
            "  non executable: {}",
            self.total_non_executable_line_count
        )?;
        writeln!(f, "  covered: {}", self.total_executable_line_covered)?;
        writeln!(f)?;
        writeln!(f, "FUNCTIONS\n")?;

        for func in &self.function_summaries {
            let func_total_line_count = func.executable_line_count + func.non_executable_line_count;
            writeln!(f, "function: {}", func.name)?;
            writeln!(f, "source: {}", func.path.display())?;
            writeln!(f, "coverage: {:.2}%", func.coverage)?;
            writeln!(f, "line count:")?;
            writeln!(f, "  total: {}", func_total_line_count)?;
            writeln!(f, "  executable: {}", func.executable_line_count)?;
            writeln!(f, "  non executable: {}", func.non_executable_line_count)?;
            writeln!(f, "  covered: {}", func.executable_line_covered)?;
            writeln!(f, "sub functions: {}\n", func.total_sub_functions)?;
        }

        Ok(())
    }
}

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
            writeln!(f, "line | hits |")?;
            writeln!(f, "---- | ---- |")?;
            for hit in &source.line_hits {
                if hit.is_executable && hit.hit_count > 0 {
                    writeln!(f, "{:4} | {:4} |{}", hit.line, hit.hit_count, hit.content)?;
                } else if hit.is_executable {
                    writeln!(f, "{:4} |    0 |{}", hit.line, hit.content)?;
                } else {
                    writeln!(f, "{:4} |      |{}", hit.line, hit.content)?;
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
    symbols: &HashMap<i64, &solc::ast::ContractDefinitionNode>,
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
    let mut total_sub_functions = 0;
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
    total_sub_functions += child_result.total_sub_functions;
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
        path: source_path,
        name: function_signature.into(),
        total_sub_functions,
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
    total_sub_functions: usize,
}

impl ResolveChildrenResult {
    fn merge_child(&mut self, other: ResolveChildrenResult) {
        self.children.extend(other.children);
        self.executable_line_count += other.executable_line_count;
        self.non_executable_line_count += other.non_executable_line_count;
        self.executable_line_covered += other.executable_line_covered;
        self.total_sub_functions += 1 + other.total_sub_functions;
    }
}

fn resolve_children(
    func_def: &solc::ast::FunctionDefinition,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    executable_lines: &HashSet<(PathBuf, usize)>,
    symbols: &HashMap<i64, &solc::ast::ContractDefinitionNode>,
    visited_ids: &mut HashSet<i64>,
    project: &Path,
) -> ResolveChildrenResult {
    let mut result = ResolveChildrenResult {
        children: Vec::new(),
        executable_line_count: 0,
        non_executable_line_count: 0,
        executable_line_covered: 0,
        total_sub_functions: 0,
    };

    let refs = collect_references_from_body(&func_def.body);
    for rid in refs {
        let Some(node) = symbols.get(&rid) else {
            continue;
        };
        let Some(child) =
            build_source_coverage(node, context, line_hits, executable_lines, project)
        else {
            continue;
        };

        if !visited_ids.insert(rid) {
            continue;
        }

        result.executable_line_count += child.executable_line_count;
        result.non_executable_line_count += child.non_executable_line_count;
        result.executable_line_covered += child.executable_line_covered;
        result.children.push(child.coverage);

        if let solc::ast::ContractDefinitionNode::FunctionDefinition(f) = node {
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

    use super::*;

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
            function counterLinked() external returns (address);
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
        runtime_code: Bytes,
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
        let runtime_code = deployment.result.output.unwrap_or_default();

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
        }

        Deployed {
            chain,
            address: target,
            runtime_code,
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
            .with_runtime_code(&deployed.runtime_code)
            .unwrap();

        let reporter = CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let reports = reporter.reports();
        assert!(
            !reports.is_empty(),
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
            .with_runtime_code(&deployed.runtime_code)
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
        let report = reporter
            .get_report("addAndSub(uint256,uint256)")
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
            .with_runtime_code(&deployed.runtime_code)
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
        let report = reporter
            .get_report("addAndSub(uint256,uint256)")
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
            .with_runtime_code(&deployed.runtime_code)
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
        let report = reporter
            .get_report("earlyReturn(uint256)")
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
            .with_runtime_code(&deployed.runtime_code)
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

        let report = reporter
            .get_report("inheritanceCall(uint256)")
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
            .with_runtime_code(&deployed.runtime_code)
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

        let report = reporter
            .get_report("libCall(uint256)")
            .expect("libCall report must be present");
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
            .with_runtime_code(&deployed.runtime_code)
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

        let report = reporter
            .get_report("libLinkedCall(uint256)")
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
}
