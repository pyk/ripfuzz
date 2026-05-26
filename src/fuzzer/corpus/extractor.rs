//! Extract literal values from build artifact ASTs for value generation.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::foundry::{Artifact, ArtifactId};

/// Literals collected from the entire project, grouped by kind.
#[derive(Debug, Clone, Default)]
pub struct ExtractedLiterals {
    pub bools: Vec<String>,
    pub numbers: Vec<String>,
    pub strings: Vec<String>,
    pub hex_strings: Vec<String>,
}

impl ExtractedLiterals {
    pub fn is_empty(&self) -> bool {
        self.bools.is_empty()
            && self.numbers.is_empty()
            && self.strings.is_empty()
            && self.hex_strings.is_empty()
    }
}

/// Walk every artifact AST and collect all literal values by kind.
pub fn extract_literals(artifacts: &HashMap<ArtifactId, Artifact>) -> ExtractedLiterals {
    artifacts
        .values()
        .par_bridge()
        .fold(ExtractedLiterals::default, |mut out, artifact| {
            visit_artifact(artifact, &mut out);
            out
        })
        .reduce(ExtractedLiterals::default, |mut a, b| {
            a.bools.extend(b.bools);
            a.numbers.extend(b.numbers);
            a.strings.extend(b.strings);
            a.hex_strings.extend(b.hex_strings);
            a
        })
}

fn visit_artifact(artifact: &Artifact, out: &mut ExtractedLiterals) {
    let ast = match artifact {
        Artifact::Contract(c) => &c.ast,
        Artifact::Interface(c) => &c.ast,
        Artifact::Library(c) => &c.ast,
        Artifact::Abstract(c) => &c.ast,
    };
    visit_source_unit(ast, out);
}

fn visit_source_unit(ast: &solc::ast::SourceUnit, out: &mut ExtractedLiterals) {
    for node in &ast.nodes {
        match node {
            solc::ast::SourceUnitNode::ContractDefinition(contract) => {
                visit_contract_definition(contract, out);
            }
            solc::ast::SourceUnitNode::FunctionDefinition(func) => {
                if let Some(body) = &func.body {
                    visit_block(body, out);
                }
            }
            solc::ast::SourceUnitNode::VariableDeclaration(var) => {
                if let Some(value) = &var.value {
                    visit_expression(value, out);
                }
            }
            _ => {}
        }
    }
}

fn visit_contract_definition(
    contract: &solc::ast::ContractDefinition,
    out: &mut ExtractedLiterals,
) {
    for node in &contract.nodes {
        match node {
            solc::ast::ContractDefinitionNode::FunctionDefinition(func) => {
                if let Some(body) = &func.body {
                    visit_block(body, out);
                }
            }
            solc::ast::ContractDefinitionNode::VariableDeclaration(var) => {
                if let Some(value) = &var.value {
                    visit_expression(value, out);
                }
            }
            _ => {}
        }
    }
}

fn visit_block(block: &solc::ast::Block, out: &mut ExtractedLiterals) {
    for stmt in &block.statements {
        visit_statement(stmt, out);
    }
}

fn visit_statement(stmt: &solc::ast::Statement, out: &mut ExtractedLiterals) {
    match stmt {
        solc::ast::Statement::ExpressionStatement(expr_stmt) => {
            visit_expression(&expr_stmt.expression, out);
        }
        solc::ast::Statement::Return(ret) => {
            if let Some(expr) = &ret.expression {
                visit_expression(expr, out);
            }
        }
        solc::ast::Statement::IfStatement(if_stmt) => {
            visit_expression(&if_stmt.condition, out);
            visit_statement(&if_stmt.true_body, out);
            if let Some(false_body) = &if_stmt.false_body {
                visit_statement(false_body, out);
            }
        }
        solc::ast::Statement::ForStatement(for_stmt) => {
            if let Some(init) = &for_stmt.initialization_expression {
                visit_expression(init, out);
            }
            visit_expression(&for_stmt.condition, out);
            if let Some(loop_expr) = &for_stmt.loop_expression {
                visit_expression(loop_expr, out);
            }
            visit_statement(&for_stmt.body, out);
        }
        solc::ast::Statement::WhileStatement(while_stmt) => {
            visit_expression(&while_stmt.condition, out);
            visit_statement(&while_stmt.body, out);
        }
        solc::ast::Statement::DoWhileStatement(do_while) => {
            visit_statement(&do_while.body, out);
            visit_expression(&do_while.condition, out);
        }
        solc::ast::Statement::VariableDeclarationStatement(var_stmt) => {
            if let Some(init) = &var_stmt.initial_value {
                visit_expression(init, out);
            }
        }
        solc::ast::Statement::Block(block) => visit_block(block, out),
        solc::ast::Statement::UncheckedBlock(unchecked) => {
            for stmt in &unchecked.statements {
                visit_statement(stmt, out);
            }
        }
        solc::ast::Statement::EmitStatement(emit) => {
            visit_function_call(&emit.event_call, out);
        }
        _ => {}
    }
}

fn visit_expression(expr: &solc::ast::Expression, out: &mut ExtractedLiterals) {
    match expr {
        solc::ast::Expression::Literal(lit) => {
            let value = lit.value.as_ref().or(lit.hex_value.as_ref()).cloned();
            let Some(value) = value else { return };
            match lit.kind {
                solc::ast::LiteralKind::Bool => out.bools.push(value),
                solc::ast::LiteralKind::Number => {
                    let computed = match lit.subdenomination.as_deref() {
                        Some(sub) => apply_subdenomination(&value, sub),
                        None => Some(value),
                    };
                    if let Some(computed) = computed {
                        out.numbers.push(computed);
                    }
                }
                solc::ast::LiteralKind::String => out.strings.push(value),
                solc::ast::LiteralKind::HexString => out.hex_strings.push(value),
                solc::ast::LiteralKind::UnicodeString => out.strings.push(value),
            }
        }
        solc::ast::Expression::Assignment(assignment) => {
            visit_expression(&assignment.left_hand_side, out);
            visit_expression(&assignment.right_hand_side, out);
        }
        solc::ast::Expression::BinaryOperation(bin_op) => {
            visit_expression(&bin_op.left_expression, out);
            visit_expression(&bin_op.right_expression, out);
        }
        solc::ast::Expression::Conditional(cond) => {
            visit_expression(&cond.condition, out);
            visit_expression(&cond.true_expression, out);
            visit_expression(&cond.false_expression, out);
        }
        solc::ast::Expression::FunctionCall(call) => {
            visit_function_call(call, out);
        }
        solc::ast::Expression::IndexAccess(idx) => {
            visit_expression(&idx.base_expression, out);
            if let Some(index) = &idx.index_expression {
                visit_expression(index, out);
            }
        }
        solc::ast::Expression::IndexRangeAccess(range) => {
            visit_expression(&range.base_expression, out);
            if let Some(start) = &range.start_expression {
                visit_expression(start, out);
            }
        }
        solc::ast::Expression::MemberAccess(member) => {
            visit_expression(&member.expression, out);
        }
        solc::ast::Expression::TupleExpression(tuple) => {
            for expr in tuple.components.iter().flatten() {
                visit_expression(expr, out);
            }
        }
        solc::ast::Expression::UnaryOperation(unary) => {
            visit_expression(&unary.sub_expression, out);
        }
        solc::ast::Expression::ExpressionStatement(stmt) => {
            visit_expression(&stmt.expression, out);
        }
        solc::ast::Expression::VariableDeclarationStatement(stmt) => {
            if let Some(init) = &stmt.initial_value {
                visit_expression(init, out);
            }
        }
        _ => {}
    }
}

fn visit_function_call(call: &solc::ast::FunctionCall, out: &mut ExtractedLiterals) {
    visit_function_call_expression(&call.expression, out);
    for arg in &call.arguments {
        visit_expression(arg, out);
    }
}

fn visit_function_call_expression(
    expr: &solc::ast::FunctionCallExpression,
    out: &mut ExtractedLiterals,
) {
    match expr {
        solc::ast::FunctionCallExpression::FunctionCall(call) => visit_function_call(call, out),
        solc::ast::FunctionCallExpression::MemberAccess(member) => {
            visit_expression(&solc::ast::Expression::MemberAccess(member.clone()), out);
        }
        solc::ast::FunctionCallExpression::NewExpression(new_expr) => {
            visit_expression(&solc::ast::Expression::NewExpression(new_expr.clone()), out);
        }
        solc::ast::FunctionCallExpression::FunctionCallOptions(options) => {
            visit_expression(&options.expression, out);
            for opt in &options.options {
                visit_expression(opt, out);
            }
        }
        _ => {}
    }
}

fn apply_subdenomination(value: &str, sub: &str) -> Option<String> {
    let factor: u128 = match sub {
        "wei" => 1,
        "gwei" => 1_000_000_000,
        "ether" => 1_000_000_000_000_000_000,
        "seconds" => 1,
        "minutes" => 60,
        "hours" => 3_600,
        "days" => 86_400,
        "weeks" => 604_800,
        _ => return None,
    };

    let factor_u = alloy_primitives::U256::from(factor);

    match value.find('.') {
        Some(dot_idx) => {
            let whole = &value[..dot_idx];
            let frac = &value[dot_idx + 1..];
            let whole_u = whole.parse::<alloy_primitives::U256>().ok()?;
            let frac_digits = frac.len() as u64;
            let frac_u = frac.parse::<alloy_primitives::U256>().ok()?;
            let pow =
                alloy_primitives::U256::from(10u64).pow(alloy_primitives::U256::from(frac_digits));
            let base = whole_u * pow + frac_u;
            let result = base * factor_u / pow;
            // checkrs: ignore[to_string_instead_of_into]
            Some(result.to_string())
        }
        None => {
            let base = value.parse::<alloy_primitives::U256>().ok()?;
            // checkrs: ignore[to_string_instead_of_into]
            Some((base * factor_u).to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundry;

    fn load_fixture() -> HashMap<ArtifactId, Artifact> {
        let project = foundry::Project::new("fixtures/target-contract-with-literals");
        let artifacts = project.load_artifacts().unwrap();
        artifacts.into_iter().map(|(k, v)| (k, v)).collect()
    }

    #[test]
    fn extracts_bool_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        assert!(
            literals.bools.contains(&"true".into()),
            "expected true in bools: {:?}",
            literals.bools
        );
        assert!(
            literals.bools.contains(&"false".into()),
            "expected false in bools: {:?}",
            literals.bools
        );
    }

    #[test]
    fn extracts_number_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected_numbers = ["42", "1000", "1337", "1", "0"];
        for expected in &expected_numbers {
            assert!(
                literals.numbers.contains(&(*expected).into()),
                "expected '{}' in numbers: {:?}",
                expected,
                literals.numbers
            );
        }
    }

    #[test]
    fn extracts_subdenomination_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        assert!(
            literals.numbers.contains(&"1000000000000000000".into()),
            "expected 1 ether (1000000000000000000) in numbers: {:?}",
            literals.numbers
        );
        assert!(
            literals.numbers.contains(&"100".into()),
            "expected 100 wei (100) in numbers: {:?}",
            literals.numbers
        );
        assert!(
            literals.numbers.contains(&"5".into()),
            "expected 5 seconds (5) in numbers: {:?}",
            literals.numbers
        );
        assert!(
            literals.numbers.contains(&"172800".into()),
            "expected 2 days (172800) in numbers: {:?}",
            literals.numbers
        );
    }

    #[test]
    fn apply_subdenomination_computes_correctly() {
        assert_eq!(
            apply_subdenomination("1", "ether"),
            Some("1000000000000000000".into())
        );
        assert_eq!(apply_subdenomination("100", "wei"), Some("100".into()));
        assert_eq!(apply_subdenomination("5", "seconds"), Some("5".into()));
        assert_eq!(apply_subdenomination("2", "days"), Some("172800".into()));
        assert_eq!(
            apply_subdenomination("0.5", "ether"),
            Some("500000000000000000".into())
        );
        assert_eq!(
            apply_subdenomination("1", "gwei"),
            Some("1000000000".into())
        );
        assert_eq!(apply_subdenomination("3", "minutes"), Some("180".into()));
        assert_eq!(apply_subdenomination("4", "hours"), Some("14400".into()));
        assert_eq!(apply_subdenomination("1", "weeks"), Some("604800".into()));
    }

    #[test]
    fn extracts_string_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected_strings = ["hello", "world", "ok"];
        for expected in &expected_strings {
            assert!(
                literals.strings.contains(&(*expected).into()),
                "expected '{}' in strings: {:?}",
                expected,
                literals.strings
            );
        }
    }

    #[test]
    fn extracts_hex_string_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected_hex = [
            "1234567890abcdef1234567890abcdef12345678",
            "deadbeef00000000000000000000000000000000000000000000000000000000",
        ];
        for expected in &expected_hex {
            assert!(
                literals.hex_strings.contains(&(*expected).into()),
                "expected '{}' in hex_strings: {:?}",
                expected,
                literals.hex_strings
            );
        }
    }
}
