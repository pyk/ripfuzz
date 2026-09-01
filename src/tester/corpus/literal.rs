//! Extraction of Solidity literal values from the harness compilation output.
//!
//! [`LiteralExtractor`] walks the AST of every compiled source and collects
//! literal values grouped by kind, so argument generation can seed its
//! distribution with the constants the harness compares against:
//!
//! - booleans
//! - unsigned and signed integers (stored at their minimum bit width and at
//!   every wider width, so a `uint8` gate value also seeds `uint256` calls)
//! - fixed-size bytes (stored at their minimum size and every larger one)
//! - addresses (checksummed hex literals and numbers that fit in 160 bits)
//! - dynamic bytes (hex string literals and trimmed number words)
//! - strings (plain, hex, and unicode string literals)
//!
//! Number subdenominations (`wei`, `gwei`, `ether`, `seconds`, `minutes`,
//! `hours`, `days`, `weeks`) are applied before the value is stored, so
//! `1 ether` lands in the pool as `1000000000000000000`. The negation of
//! every number literal is stored too, so gates that compare against
//! negative values are reachable.

use std::collections::HashMap;

use alloy_primitives::{Address, Bytes, I256, U256};
use solc::StandardJSONOutput;

/// Literals collected from the harness sources, grouped to map directly to
/// [`DynSolValue`](alloy_dyn_abi::DynSolValue) variants.
#[derive(Debug, Clone, Default)]
pub struct LiteralExtractor {
    bools: Vec<bool>,
    uints: HashMap<usize, Vec<U256>>,
    ints: HashMap<usize, Vec<I256>>,
    fixed: HashMap<usize, Vec<U256>>,
    addresses: Vec<Address>,
    byte_strings: Vec<Bytes>,
    strings: Vec<String>,
}

impl LiteralExtractor {
    /// Create an empty extractor.
    pub fn new() -> Self {
        Self::default()
    }

    /// Walk the AST of every source in the solc output and collect all
    /// literal values by kind.
    pub fn from_output(output: &StandardJSONOutput) -> Self {
        let mut out = Self::default();
        for source in output.sources.values() {
            if let Some(ast) = source.ast.as_ref() {
                visit_source_unit(ast, &mut out);
            }
        }
        out
    }

    /// Whether no literal was extracted.
    pub fn is_empty(&self) -> bool {
        self.bools.is_empty()
            && self.uints.is_empty()
            && self.ints.is_empty()
            && self.addresses.is_empty()
            && self.fixed.is_empty()
            && self.byte_strings.is_empty()
            && self.strings.is_empty()
    }

    /// The boolean literals.
    pub fn bools(&self) -> &[bool] {
        &self.bools
    }

    /// The unsigned literals available at the given bit width.
    pub fn uint(&self, bits: usize) -> &[U256] {
        self.uints.get(&bits).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The signed literals available at the given bit width.
    pub fn int(&self, bits: usize) -> &[I256] {
        self.ints.get(&bits).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The fixed-size bytes literals available at the given byte size,
    /// stored as their numeric value.
    pub fn fixed_bytes(&self, size: usize) -> &[U256] {
        self.fixed.get(&size).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// The address literals.
    pub fn addresses(&self) -> &[Address] {
        &self.addresses
    }

    /// The dynamic bytes literals.
    pub fn bytes(&self) -> &[Bytes] {
        &self.byte_strings
    }

    /// The string literals.
    pub fn strings(&self) -> &[String] {
        &self.strings
    }
}

fn visit_source_unit(ast: &solc::ast::SourceUnit, out: &mut LiteralExtractor) {
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

fn visit_contract_definition(contract: &solc::ast::ContractDefinition, out: &mut LiteralExtractor) {
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

fn visit_block(block: &solc::ast::Block, out: &mut LiteralExtractor) {
    for stmt in &block.statements {
        visit_statement(stmt, out);
    }
}

fn visit_statement(stmt: &solc::ast::Statement, out: &mut LiteralExtractor) {
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

fn visit_expression(expr: &solc::ast::Expression, out: &mut LiteralExtractor) {
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

fn parse_literal_number(lit: &solc::ast::Literal) -> Option<U256> {
    let value = if lit.kind == solc::ast::LiteralKind::HexString {
        lit.hex_value.as_ref().or(lit.value.as_ref()).cloned()
    } else {
        lit.value.as_ref().or(lit.hex_value.as_ref()).cloned()
    };
    let value = value?;
    match lit.subdenomination.as_deref() {
        Some(sub) => apply_subdenomination(&value, sub),
        None => parse_number_literal(&value),
    }
}

fn extract_literal(lit: &solc::ast::Literal, out: &mut LiteralExtractor) {
    match lit.kind {
        solc::ast::LiteralKind::Bool => {
            let Some(value) = lit.value.as_ref().or(lit.hex_value.as_ref()).cloned() else {
                return;
            };
            out.bools.push(value == "true");
        }
        solc::ast::LiteralKind::Number => {
            let Some(u) = parse_literal_number(lit) else {
                return;
            };

            // Address compatible: values that fit in 160 bits.
            let bytes = u.to_be_bytes::<32>();
            if bytes[..12].iter().all(|&b| b == 0) {
                let addr = Address::from_slice(&bytes[12..]);
                out.addresses.push(addr);
                out.byte_strings
                    .push(Bytes::copy_from_slice(addr.as_slice()));
            }

            push_uint(u, &mut out.uints);
            out.byte_strings.push(u256_to_be_bytes_trimmed(u));
            if let Ok(i) = I256::try_from(u)
                && let Some(negated) = i.checked_neg()
            {
                push_int(i, &mut out.ints);
                push_int(negated, &mut out.ints);
                out.byte_strings.push(i256_to_be_bytes_trimmed(i));
                out.byte_strings.push(i256_to_be_bytes_trimmed(negated));
            }

            let word = u.to_be_bytes::<32>();
            let leading_zeros = word.iter().take_while(|&&b| b == 0).count();
            let min_size = (32 - leading_zeros).max(1);
            push_fixed_bytes(u, min_size, &mut out.fixed);
            out.byte_strings.push(u256_to_be_bytes_trimmed(u));
        }
        solc::ast::LiteralKind::String | solc::ast::LiteralKind::UnicodeString => {
            let Some(value) = lit.value.as_ref().or(lit.hex_value.as_ref()).cloned() else {
                return;
            };
            out.strings.push(value.clone());
            out.byte_strings.push(Bytes::from(value.into_bytes()));
        }
        solc::ast::LiteralKind::HexString => {
            let Some(value) = lit.hex_value.as_ref().or(lit.value.as_ref()).cloned() else {
                return;
            };
            let Ok(bytes) = hex::decode(&value) else {
                return;
            };
            out.byte_strings.push(Bytes::from(bytes.clone()));

            if bytes.len() == 20 {
                let addr = Address::from_slice(&bytes);
                out.addresses.push(addr);
                out.byte_strings
                    .push(Bytes::copy_from_slice(addr.as_slice()));
            }

            if bytes.len() <= 32 {
                let mut word = [0u8; 32];
                word[..bytes.len()].copy_from_slice(&bytes);
                let min_size = bytes.len().max(1);
                let mut num_word = [0u8; 32];
                num_word[32 - bytes.len()..].copy_from_slice(&bytes);
                let u = U256::from_be_bytes(num_word);
                push_fixed_bytes(u, min_size, &mut out.fixed);
                out.byte_strings.push(Bytes::from(word.to_vec()));
                push_uint(u, &mut out.uints);
                out.byte_strings.push(u256_to_be_bytes_trimmed(u));
                if let Ok(i) = I256::try_from(u)
                    && let Some(negated) = i.checked_neg()
                {
                    push_int(i, &mut out.ints);
                    push_int(negated, &mut out.ints);
                    out.byte_strings.push(i256_to_be_bytes_trimmed(i));
                    out.byte_strings.push(i256_to_be_bytes_trimmed(negated));
                }
            }
        }
    }
}

fn push_uint(value: U256, uints: &mut HashMap<usize, Vec<U256>>) {
    let magnitude_bits = if value == U256::ZERO {
        0
    } else {
        256 - value.leading_zeros()
    };
    let min_bits = magnitude_bits.div_ceil(8) * 8;
    let min_bits = min_bits.max(8);
    for bits in (min_bits..=256).step_by(8) {
        uints.entry(bits).or_default().push(value);
    }
}

fn push_int(value: I256, ints: &mut HashMap<usize, Vec<I256>>) {
    let min_bits = if value == I256::ZERO {
        8
    } else {
        let raw = value.into_raw();
        let abs_u256 = if value.is_negative() {
            (!raw).wrapping_add(U256::ONE)
        } else {
            raw
        };
        let magnitude_bits = 256 - abs_u256.leading_zeros();
        if value > I256::ZERO {
            (magnitude_bits + 1).div_ceil(8) * 8
        } else if abs_u256.count_ones() == 1 {
            magnitude_bits.div_ceil(8) * 8
        } else {
            (magnitude_bits + 1).div_ceil(8) * 8
        }
    };
    let min_bits = min_bits.clamp(8, 256);
    for bits in (min_bits..=256).step_by(8) {
        ints.entry(bits).or_default().push(value);
    }
}

fn push_fixed_bytes(value: U256, min_size: usize, fixed: &mut HashMap<usize, Vec<U256>>) {
    for size in min_size..=32 {
        fixed.entry(size).or_default().push(value);
    }
}

fn u256_to_be_bytes_trimmed(value: U256) -> Bytes {
    let bytes = value.to_be_bytes::<32>();
    let leading_zeros = bytes.iter().take_while(|&&b| b == 0x00).count();
    if leading_zeros == 32 {
        return Bytes::from_static(&[0x00]);
    }
    Bytes::copy_from_slice(&bytes[leading_zeros..])
}

fn i256_to_be_bytes_trimmed(value: I256) -> Bytes {
    if value == I256::ZERO {
        return Bytes::from_static(&[0x00]);
    }
    let raw = value.into_raw().to_be_bytes::<32>();
    if value.is_negative() {
        let leading_ones = raw.iter().take_while(|&&b| b == 0xFF).count();
        let start = if leading_ones == 32 {
            31
        } else if raw[leading_ones] < 0x80 {
            leading_ones.saturating_sub(1)
        } else {
            leading_ones
        };
        Bytes::copy_from_slice(&raw[start..])
    } else {
        let leading_zeros = raw.iter().take_while(|&&b| b == 0x00).count();
        Bytes::copy_from_slice(&raw[leading_zeros..])
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

fn visit_function_call(call: &solc::ast::FunctionCall, out: &mut LiteralExtractor) {
    visit_function_call_expression(&call.expression, out);
    for arg in &call.arguments {
        visit_expression(arg, out);
    }
}

fn visit_function_call_expression(
    expr: &solc::ast::FunctionCallExpression,
    out: &mut LiteralExtractor,
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
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_primitives::I256;
    use fastrand::Rng;

    use super::*;
    use crate::tester::corpus::rvg::RandomValueGenerator;

    /// A signed literal pushed into a narrow-width pool must be reachable
    /// from the generator: the range check compares against the true minimum
    /// of the width, not its positive sign-bit value.
    #[test]
    fn signed_literals_are_reachable_for_narrow_widths() {
        let mut literals = LiteralExtractor::default();
        push_int(I256::try_from(-3).unwrap(), &mut literals.ints);

        let mut rng = Rng::new();
        let mut hits = 0;
        for _ in 0..1000 {
            let mut generator = RandomValueGenerator::new(&mut rng, &literals);
            let DynSolValue::Int(value, _) = generator.value(&DynSolType::Int(8)) else {
                panic!("expected an int");
            };
            if value == I256::try_from(-3).unwrap() {
                hits += 1;
            }
        }
        assert!(
            hits > 100,
            "the literal branch must return the pool value, got {hits}/1000"
        );
    }
}
