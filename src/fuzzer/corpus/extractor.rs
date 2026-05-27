//! Extract literal values from build artifact ASTs for value generation.

use std::collections::HashMap;

use alloy_primitives::{Address, FixedBytes, I256, U256};
use rayon::prelude::*;

use crate::foundry::{Artifact, ArtifactId};

/// Literals collected from the entire project, grouped to map directly to
/// [`DynSolValue`] variants.
#[derive(Debug, Clone, Default)]
pub struct ExtractedLiterals {
    pub bool: Vec<bool>,
    pub uint: HashMap<usize, Vec<U256>>,
    pub int: HashMap<usize, Vec<I256>>,
    pub fixed_bytes: Vec<FixedBytes<32>>,
    pub address: Vec<Address>,
    pub bytes: Vec<Vec<u8>>,
    pub string: Vec<String>,
}

impl ExtractedLiterals {
    pub fn is_empty(&self) -> bool {
        self.bool.is_empty()
            && self.uint.is_empty()
            && self.int.is_empty()
            && self.address.is_empty()
            && self.fixed_bytes.is_empty()
            && self.bytes.is_empty()
            && self.string.is_empty()
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
            a.bool.extend(b.bool);
            for (bits, vals) in b.uint {
                a.uint.entry(bits).or_default().extend(vals);
            }
            for (bits, vals) in b.int {
                a.int.entry(bits).or_default().extend(vals);
            }
            a.address.extend(b.address);
            a.fixed_bytes.extend(b.fixed_bytes);
            a.bytes.extend(b.bytes);
            a.string.extend(b.string);
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
        solc::ast::Expression::Literal(lit) => extract_literal(lit, out),
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

fn extract_literal(lit: &solc::ast::Literal, out: &mut ExtractedLiterals) {
    let value = if lit.kind == solc::ast::LiteralKind::HexString {
        lit.hex_value.as_ref().or(lit.value.as_ref()).cloned()
    } else {
        lit.value.as_ref().or(lit.hex_value.as_ref()).cloned()
    };
    let Some(value) = value else { return };

    match lit.kind {
        solc::ast::LiteralKind::Bool => {
            out.bool.push(value == "true");
        }
        solc::ast::LiteralKind::Number => {
            let computed = match lit.subdenomination.as_deref() {
                Some(sub) => apply_subdenomination(&value, sub),
                None => parse_number_literal(&value),
            };
            let Some(u) = computed else { return };

            push_uint(u, &mut out.uint);
            push_int(u, &mut out.int);

            // Address compatible: values that fit in 160 bits.
            let bytes = u.to_be_bytes::<32>();
            if bytes[..12].iter().all(|&b| b == 0) {
                out.address.push(Address::from_slice(&bytes[12..]));
            }
        }
        solc::ast::LiteralKind::String | solc::ast::LiteralKind::UnicodeString => {
            out.string.push(value.clone());
            out.bytes.push(value.into_bytes());
        }
        solc::ast::LiteralKind::HexString => {
            let Ok(bytes) = hex::decode(&value) else {
                return;
            };
            out.bytes.push(bytes.clone());

            if bytes.len() == 20 {
                out.address.push(Address::from_slice(&bytes));
            }

            if bytes.len() <= 32 {
                let mut word = [0u8; 32];
                word[..bytes.len()].copy_from_slice(&bytes);
                out.fixed_bytes.push(FixedBytes::from(word));

                let mut num_word = [0u8; 32];
                num_word[32 - bytes.len()..].copy_from_slice(&bytes);
                let u = U256::from_be_bytes(num_word);
                push_uint(u, &mut out.uint);
                push_int(u, &mut out.int);
            }
        }
    }
}

fn push_uint(value: U256, uint: &mut HashMap<usize, Vec<U256>>) {
    let magnitude_bits = if value == U256::ZERO {
        0
    } else {
        256 - value.leading_zeros()
    };
    let min_bits = magnitude_bits.div_ceil(8) * 8;
    let min_bits = min_bits.max(8);
    for bits in (min_bits..=256).step_by(8) {
        uint.entry(bits).or_default().push(value);
    }
}

fn push_int(value: U256, int: &mut HashMap<usize, Vec<I256>>) {
    let Ok(i) = I256::try_from(value) else { return };
    let magnitude_bits = if value == U256::ZERO {
        0
    } else {
        256 - value.leading_zeros()
    };
    let min_bits = (magnitude_bits + 1).div_ceil(8) * 8;
    let min_bits = min_bits.clamp(8, 256);
    for bits in (min_bits..=256).step_by(8) {
        int.entry(bits).or_default().push(i);
    }
}

fn parse_number_literal(val: &str) -> Option<U256> {
    let trimmed = val.trim();
    if trimmed.starts_with("0x") || trimmed.starts_with("0X") {
        U256::from_str_radix(&trimmed[2..], 16).ok()
    } else if let Some((base, exp)) = trimmed.split_once(['e', 'E']) {
        let exp = exp.parse::<u64>().ok()?;
        let (base_val, frac_len) = match base.split_once('.') {
            Some((whole, frac)) => {
                let whole_u = whole.parse::<U256>().ok()?;
                let frac_len = frac.len() as u64;
                let frac_u = frac.parse::<U256>().ok()?;
                let pow = U256::from(10u64).pow(U256::from(frac_len));
                (whole_u * pow + frac_u, frac_len)
            }
            None => (base.parse::<U256>().ok()?, 0),
        };
        let num = base_val * U256::from(10u64).pow(U256::from(exp));
        let den = U256::from(10u64).pow(U256::from(frac_len));
        Some(num / den)
    } else {
        U256::from_str_radix(trimmed, 10).ok()
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

fn apply_subdenomination(value: &str, sub: &str) -> Option<U256> {
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

    let factor_u = U256::from(factor);

    match value.find('.') {
        Some(dot_idx) => {
            let whole = &value[..dot_idx];
            let frac = &value[dot_idx + 1..];
            let whole_u = whole.parse::<U256>().ok()?;
            let frac_digits = frac.len() as u64;
            let frac_u = frac.parse::<U256>().ok()?;
            let pow = U256::from(10u64).pow(U256::from(frac_digits));
            let base = whole_u * pow + frac_u;
            Some(base * factor_u / pow)
        }
        None => {
            let base = value.parse::<U256>().ok()?;
            Some(base * factor_u)
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
        assert_eq!(literals.bool, vec![true, false]);
    }

    #[test]
    fn extracts_uint256_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected = vec![
            // useNumbers()
            U256::from(0),
            U256::from(1),
            U256::from(42),
            U256::from(1000),
            U256::from(1337),
            // useNumberFormats()
            U256::from(0x1234),                  // 0x1234
            U256::from(1000000000000000000u128), // 1e18
            U256::from(1000000),                 // 1_000_000
            U256::from_str_radix(
                "115792089237316195423570985008687907853269984665640564039457584007913129639935",
                10,
            )
            .unwrap(), // uint256 max
            // useSubdenominations()
            U256::from(1),                       // 1 wei
            U256::from(100),                     // 100 wei
            U256::from(1000000000u128),          // 1 gwei
            U256::from(1000000000000000000u128), // 1 ether
            U256::from(500000000000000000u128),  // 0.5 ether
            U256::from(5),                       // 5 seconds
            U256::from(60),                      // 1 minutes
            U256::from(3600),                    // 1 hours
            U256::from(172800),                  // 2 days
            U256::from(604800),                  // 1 weeks
            // useHexStrings()
            U256::from(0),      // hex""
            U256::from(0),      // hex"00"
            U256::from(0x1234), // hex'1234'
            U256::from_str_radix("1234567890abcdef1234567890abcdef12345678", 16).unwrap(), // 20-byte hex
            U256::from_str_radix(
                "deadbeef00000000000000000000000000000000000000000000000000000000",
                16,
            )
            .unwrap(), // 32-byte hex
            // useAddresses()
            U256::from_str_radix("abCDEF1234567890ABcDEF1234567890aBCDeF12", 16).unwrap(), // address literal
            // invariant_check()
            U256::from(0), // num >= 0
        ];
        assert_eq!(
            literals.uint.get(&256).unwrap().clone(),
            expected,
            "uint256 group mismatch"
        );
    }

    #[test]
    fn extracts_uint128_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected = vec![
            // useNumbers()
            U256::from(0),
            U256::from(1),
            U256::from(42),
            U256::from(1000),
            U256::from(1337),
            // useNumberFormats()
            U256::from(0x1234),                  // 0x1234
            U256::from(1000000000000000000u128), // 1e18
            U256::from(1000000),                 // 1_000_000
            // uint256 max omitted - does not fit in uint128
            // useSubdenominations()
            U256::from(1),                       // 1 wei
            U256::from(100),                     // 100 wei
            U256::from(1000000000u128),          // 1 gwei
            U256::from(1000000000000000000u128), // 1 ether
            U256::from(500000000000000000u128),  // 0.5 ether
            U256::from(5),                       // 5 seconds
            U256::from(60),                      // 1 minutes
            U256::from(3600),                    // 1 hours
            U256::from(172800),                  // 2 days
            U256::from(604800),                  // 1 weeks
            // useHexStrings()
            U256::from(0),      // hex""
            U256::from(0),      // hex"00"
            U256::from(0x1234), // hex'1234'
            // 20-byte hex omitted - 160 bits, does not fit in uint128
            // 32-byte hex omitted - > 2^255, does not fit in uint128
            // useAddresses()
            // address literal omitted - 160 bits, does not fit in uint128
            // invariant_check()
            U256::from(0), // num >= 0
        ];
        assert_eq!(
            literals.uint.get(&128).unwrap().clone(),
            expected,
            "uint128 group mismatch"
        );
    }

    #[test]
    fn extracts_uint8_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected = vec![
            // useNumbers()
            U256::from(0),  // 0
            U256::from(1),  // 1
            U256::from(42), // 42
            // 1000 omitted - > 255
            // 1337 omitted - > 255
            // useNumberFormats() - all omitted
            // useSubdenominations()
            U256::from(1),   // 1 wei
            U256::from(100), // 100 wei
            // 1 gwei omitted - > 255
            // 1 ether omitted - > 255
            // 0.5 ether omitted - > 255
            U256::from(5),  // 5 seconds
            U256::from(60), // 1 minutes
            // 1 hours omitted - > 255
            // 2 days omitted - > 255
            // 1 weeks omitted - > 255
            // useHexStrings()
            U256::from(0), // hex""
            U256::from(0), // hex"00"
            // hex'1234' omitted - > 255
            // 20-byte hex omitted - > 255
            // 32-byte hex omitted - > 255
            // useAddresses() - omitted
            // invariant_check()
            U256::from(0), // num >= 0
        ];
        assert_eq!(
            literals.uint.get(&8).unwrap().clone(),
            expected,
            "uint8 group mismatch"
        );
    }

    #[test]
    fn extracts_number_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected_numbers = [
            U256::from(42),
            U256::from(1000),
            U256::from(1337),
            U256::from(1),
            U256::from(0),
        ];
        for expected in &expected_numbers {
            assert!(
                literals.uint.get(&256).unwrap().contains(expected),
                "expected '{}' in uint256: {:?}",
                expected,
                literals.uint.get(&256).unwrap()
            );
        }
        // Small values also appear in smaller groups.
        assert!(literals.uint.get(&8).unwrap().contains(&U256::from(42)));
        assert!(literals.uint.get(&8).unwrap().contains(&U256::from(1)));
        assert!(literals.uint.get(&8).unwrap().contains(&U256::from(0)));
    }

    #[test]
    fn extracts_int256_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        assert!(
            literals
                .int
                .get(&256)
                .unwrap()
                .contains(&I256::try_from(U256::from(42)).unwrap()),
            "expected 42 in int256: {:?}",
            literals.int.get(&256).unwrap()
        );
        // uint256 max should NOT be in int256
        let uint256_max = U256::from_str_radix(
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
            10,
        )
        .unwrap();
        assert!(
            !literals
                .int
                .get(&256)
                .unwrap()
                .contains(&I256::from_raw(uint256_max)),
            "expected uint256 max NOT in int256"
        );
    }

    #[test]
    fn extracts_subdenomination_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        assert!(
            literals
                .uint
                .get(&256)
                .unwrap()
                .contains(&U256::from(1000000000000000000u128)),
            "expected 1 ether (1000000000000000000) in uint256: {:?}",
            literals.uint.get(&256).unwrap()
        );
        assert!(
            literals.uint.get(&256).unwrap().contains(&U256::from(100)),
            "expected 100 wei (100) in uint256: {:?}",
            literals.uint.get(&256).unwrap()
        );
        assert!(
            literals.uint.get(&256).unwrap().contains(&U256::from(5)),
            "expected 5 seconds (5) in uint256: {:?}",
            literals.uint.get(&256).unwrap()
        );
        assert!(
            literals
                .uint
                .get(&256)
                .unwrap()
                .contains(&U256::from(172800)),
            "expected 2 days (172800) in uint256: {:?}",
            literals.uint.get(&256).unwrap()
        );
    }

    #[test]
    fn apply_subdenomination_computes_correctly() {
        assert_eq!(
            apply_subdenomination("1", "ether"),
            Some(U256::from(1000000000000000000u128))
        );
        assert_eq!(apply_subdenomination("100", "wei"), Some(U256::from(100)));
        assert_eq!(apply_subdenomination("5", "seconds"), Some(U256::from(5)));
        assert_eq!(apply_subdenomination("2", "days"), Some(U256::from(172800)));
        assert_eq!(
            apply_subdenomination("0.5", "ether"),
            Some(U256::from(500000000000000000u128))
        );
        assert_eq!(
            apply_subdenomination("1", "gwei"),
            Some(U256::from(1000000000u128))
        );
        assert_eq!(apply_subdenomination("3", "minutes"), Some(U256::from(180)));
        assert_eq!(apply_subdenomination("4", "hours"), Some(U256::from(14400)));
        assert_eq!(
            apply_subdenomination("1", "weeks"),
            Some(U256::from(604800))
        );
    }

    #[test]
    fn extracts_string_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected_strings = ["hello", "world", "ok"];
        for expected in &expected_strings {
            assert!(
                literals.string.contains(&(*expected).into()),
                "expected '{}' in string: {:?}",
                expected,
                literals.string
            );
        }
    }

    #[test]
    fn extracts_hex_string_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);

        // 20-byte hex string left-padded to 32 bytes
        let mut word1 = [0u8; 32];
        let bytes1 = hex::decode("1234567890abcdef1234567890abcdef12345678").unwrap();
        word1[..bytes1.len()].copy_from_slice(&bytes1);
        assert!(
            literals.fixed_bytes.contains(&FixedBytes::from(word1)),
            "expected 20-byte hex in fixed_bytes: {:?}",
            literals.fixed_bytes
        );

        // 32-byte hex string
        let bytes2 =
            hex::decode("deadbeef00000000000000000000000000000000000000000000000000000000")
                .unwrap();
        let mut word2 = [0u8; 32];
        word2.copy_from_slice(&bytes2);
        assert!(
            literals.fixed_bytes.contains(&FixedBytes::from(word2)),
            "expected 32-byte hex in fixed_bytes: {:?}",
            literals.fixed_bytes
        );
    }

    #[test]
    fn extracts_address_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        let expected =
            Address::from_slice(&hex::decode("abCDEF1234567890ABcDEF1234567890aBCDeF12").unwrap());
        assert!(
            literals.address.contains(&expected),
            "expected address in address: {:?}",
            literals.address
        );
    }

    #[test]
    fn extracts_bytes_literals() {
        let artifacts = load_fixture();
        let literals = extract_literals(&artifacts);
        assert!(
            literals
                .bytes
                .contains(&hex::decode("1234567890abcdef1234567890abcdef12345678").unwrap()),
            "expected hex bytes in bytes: {:?}",
            literals.bytes
        );
        assert!(
            literals.bytes.contains(&"hello".as_bytes().to_vec()),
            "expected string bytes in bytes: {:?}",
            literals.bytes
        );
    }
}
