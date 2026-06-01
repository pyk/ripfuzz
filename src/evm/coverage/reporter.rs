//! Coverage reporter for building human-readable coverage reports.
//!
//! [`CoverageReporter`] is the presentation layer of the coverage reporting
//! pipeline. It takes three inputs:
//!
//! 1. [`SharedCoverage`] - the global, merged coverage map containing raw PC
//!    hit counts from evert fuzzer execution during a campaign.
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
//! In short: `CoverageContext` knows what code was hit and where it lives in
//! source; `CoverageReporter` decides how to present that information.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;

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
    pub total_lines: usize,
    pub total_covered_lines: usize,
    pub total_coverage: f64,
    pub path: PathBuf,
    pub name: String,
    pub total_sub_functions: usize,
}

/// A detailed coverage report for a single target function.
#[derive(Debug, Clone)]
pub struct FunctionReport {
    pub total_lines: usize,
    pub total_covered_lines: usize,
    pub total_coverage: f64,
    pub path: PathBuf,
    pub name: String,
    pub total_sub_functions: usize,
    pub source_coverages: Vec<SourceCoverage>,
}

/// A summary report for the entire campaign.
#[derive(Debug, Clone)]
pub struct SummaryReport {
    pub total_lines: usize,
    pub total_covered_lines: usize,
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
        let total_lines = reports.iter().map(|r| r.total_lines).sum();
        let total_covered_lines = reports.iter().map(|r| r.total_covered_lines).sum();
        let total_coverage = if total_lines > 0 {
            (total_covered_lines as f64 / total_lines as f64) * 100.0
        } else {
            0.0
        };

        let function_summaries = reports
            .into_iter()
            .map(|r| FunctionSummaryReport {
                total_lines: r.total_lines,
                total_covered_lines: r.total_covered_lines,
                total_coverage: r.total_coverage,
                path: r.path,
                name: r.name,
                total_sub_functions: r.total_sub_functions,
            })
            .collect();

        SummaryReport {
            total_lines,
            total_covered_lines,
            total_coverage,
            function_summaries,
        }
    }

    /// Return the detailed report for a specific target function signature.
    pub fn get_report(&self, function_signature: &str) -> Option<FunctionReport> {
        let line_hits = self.context.build_line_hits(&self.coverage);
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
            &symbols,
            function_signature,
        )
    }

    /// Return detailed reports for all target functions.
    pub fn reports(&self) -> Vec<FunctionReport> {
        let line_hits = self.context.build_line_hits(&self.coverage);
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
            if let Some(report) =
                build_function_report(func_def, &self.context, &line_hits, &symbols, &sig)
            {
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
        let uncovered = self.total_lines.saturating_sub(self.total_covered_lines);
        let pct = self.total_coverage;

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "total lines: {}", self.total_lines)?;
        writeln!(f, "total covered lines: {}", self.total_covered_lines)?;
        writeln!(f, "total uncovered lines: {}", uncovered)?;
        writeln!(f, "coverage: {:.2}%\n", pct)?;
        writeln!(f, "FUNCTIONS\n")?;

        for func in &self.function_summaries {
            writeln!(f, "function: {}", func.name)?;
            writeln!(f, "source: {}", func.path.display())?;
            writeln!(f, "coverage: {:.2}%", func.total_coverage)?;
            writeln!(
                f,
                "lines: {}/{}",
                func.total_covered_lines, func.total_lines
            )?;
            writeln!(f, "sub functions: {}\n", func.total_sub_functions)?;
        }

        Ok(())
    }
}

impl fmt::Display for FunctionReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let uncovered = self.total_lines.saturating_sub(self.total_covered_lines);
        let pct = self.total_coverage;

        writeln!(f, "COVERAGE REPORT STATS\n")?;
        writeln!(f, "total lines: {}", self.total_lines)?;
        writeln!(f, "total covered lines: {}", self.total_covered_lines)?;
        writeln!(f, "total uncovered lines: {}", uncovered)?;
        writeln!(f, "coverage: {:.2}%\n", pct)?;
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
                if hit.hit_count > 0 {
                    writeln!(f, "{:4} | {:4} |{}", hit.line, hit.hit_count, hit.content)?;
                } else {
                    writeln!(f, "     |      |{}", hit.content)?;
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
        let content = file
            .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
            .unwrap_or("")
            .into();
        hits.push(LineHits {
            line,
            hit_count,
            content,
        });
    }

    let source_coverage = SourceCoverage {
        symbol: function_signature.into(),
        source: source_path.clone(),
        start_line,
        end_line,
        line_hits: hits,
        project: context.project_paths().first().cloned().unwrap_or_default(),
    };

    let mut source_coverages = vec![source_coverage];
    let mut total_covered_lines = covered_lines;
    let mut total_lines = total_lines;
    let mut total_sub_functions = 0;
    let mut visited_ids = HashSet::new();
    visited_ids.insert(func_def.id);

    let child_result = resolve_children(func_def, context, line_hits, symbols, &mut visited_ids);

    total_lines += child_result.total_lines;
    total_covered_lines += child_result.total_covered_lines;
    total_sub_functions += child_result.total_sub_functions;
    source_coverages.extend(child_result.children);

    let total_coverage = if total_lines > 0 {
        (total_covered_lines as f64 / total_lines as f64) * 100.0
    } else {
        0.0
    };

    Some(FunctionReport {
        total_lines,
        total_covered_lines,
        total_coverage,
        path: source_path,
        name: function_signature.into(),
        total_sub_functions,
        source_coverages,
    })
}

fn build_source_coverage(
    node: &solc::ast::ContractDefinitionNode,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
) -> Option<SourceCoverage> {
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

            let mut hits = Vec::new();
            for line in start_line..=end_line {
                let hit_count = line_hits
                    .get(&(source_path.clone(), line))
                    .copied()
                    .unwrap_or(0);
                let content = file
                    .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
                    .unwrap_or("")
                    .into();
                hits.push(LineHits {
                    line,
                    hit_count,
                    content,
                });
            }

            Some(SourceCoverage {
                symbol: build_signature_from_ast(func),
                source: source_path,
                start_line,
                end_line,
                line_hits: hits,
                project: context.project_paths().first().cloned().unwrap_or_default(),
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

            let mut hits = Vec::new();
            for line in start_line..=end_line {
                let content = file
                    .and_then(|f| f.content.lines().nth(line.saturating_sub(1)))
                    .unwrap_or("")
                    .into();
                hits.push(LineHits {
                    line,
                    hit_count: 0,
                    content,
                });
            }

            Some(SourceCoverage {
                symbol: var.name.clone(),
                source: source_path,
                start_line,
                end_line,
                line_hits: hits,
                project: context.project_paths().first().cloned().unwrap_or_default(),
            })
        }
        _ => None,
    }
}

struct ResolveChildrenResult {
    children: Vec<SourceCoverage>,
    total_lines: usize,
    total_covered_lines: usize,
    total_sub_functions: usize,
}

fn resolve_children(
    func_def: &solc::ast::FunctionDefinition,
    context: &CoverageContext,
    line_hits: &HashMap<(PathBuf, usize), u64>,
    symbols: &HashMap<i64, &solc::ast::ContractDefinitionNode>,
    visited_ids: &mut HashSet<i64>,
) -> ResolveChildrenResult {
    let mut children = Vec::new();
    let mut total_lines = 0;
    let mut total_covered_lines = 0;
    let mut total_sub_functions = 0;

    let refs = collect_references_from_body(&func_def.body);
    for rid in refs {
        let Some(node) = symbols.get(&rid) else {
            continue;
        };
        let Some(child) = build_source_coverage(node, context, line_hits) else {
            continue;
        };

        if !visited_ids.insert(rid) {
            continue;
        }

        total_lines += child.line_hits.len();
        total_covered_lines += child.line_hits.iter().filter(|h| h.hit_count > 0).count();
        children.push(child);

        if let solc::ast::ContractDefinitionNode::FunctionDefinition(f) = node {
            let sub = resolve_children(f, context, line_hits, symbols, visited_ids);
            total_sub_functions += 1 + sub.total_sub_functions;
            total_lines += sub.total_lines;
            total_covered_lines += sub.total_covered_lines;
            children.extend(sub.children);
        }
    }

    ResolveChildrenResult {
        children,
        total_lines,
        total_covered_lines,
        total_sub_functions,
    }
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

    struct Deployed {
        chain: Chain,
        address: revm::primitives::Address,
        runtime_code: Bytes,
    }

    fn deploy_and_setup(contract: &Contract) -> Deployed {
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();
        let runtime_code = deployment.result.output.unwrap_or_default();

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = crate::evm::chain::SetupInput::new(target).calldata(setup_data);
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
        let context = super::CoverageContext::from_project(&project)
            .unwrap()
            .with_runtime_code(&deployed.runtime_code)
            .unwrap();

        let reporter = super::CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let reports = reporter.reports();
        assert!(
            !reports.is_empty(),
            "coverage report should contain reports even when build artifacts include interfaces"
        );
    }

    /// Coverage report for a contract with internal functions that read and
    /// write storage must produce a valid display output.
    #[test]
    fn coverage_report_internal_functions() {
        let contract = load_coverage_fixture(
            "src/CoverageReportInternalFunctions.sol:CoverageReportInternalFunctions",
        );
        let mut deployed = deploy_and_setup(&contract);

        let global = SharedCoverage::new();
        let txs = vec![
            Transaction::new(deployed.address).calldata(Bytes::from(
                CoverageReportInternalFunctions::add_and_subCall::new((
                    U256::from(123),
                    U256::from(123),
                ))
                .abi_encode(),
            )),
        ];
        let exec = deployed.chain.exec(&txs).unwrap();
        let coverage = exec.coverage.expect("coverage must be present");
        global.merge(&coverage);

        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let context = super::CoverageContext::from_project(&project)
            .unwrap()
            .with_runtime_code(&deployed.runtime_code)
            .unwrap();

        let reporter = super::CoverageReporter::new()
            .coverage(global)
            .target_functions(contract.target_functions)
            .context(context);

        let expected_file = "fixtures/target-contract-coverage/expected/CoverageReportInternalFunctions_add_and_sub.txt";
        let report = reporter
            .get_report("add_and_sub(uint256,uint256)")
            .expect("add_and_sub report must be present");
        let formatted = format!("{report}");
        let expected = fs::read_to_string(expected_file)
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "coverage report output must match expected"
        );
    }
}
