//! Block / state manipulation cheatcodes.

use revm::{
    context::{BlockEnv, ContextSetters},
    context_interface::ContextTr,
    database::InMemoryDB,
    interpreter::CallOutcome,
};

use crate::chain::cheatcodes::{CheatcodeInspector, decode_u256_arg, dummy_success};

/// `warp(uint256)` — set block timestamp.
pub const WARP_SELECTOR: [u8; 4] = [0xe5, 0xd6, 0xbf, 0x02];
/// `roll(uint256)` — set block number.
pub const ROLL_SELECTOR: [u8; 4] = [0x1f, 0x7b, 0x4f, 0x30];
/// `fee(uint256)` — set base fee.
pub const FEE_SELECTOR: [u8; 4] = [0x39, 0xb3, 0x7a, 0xb0];
/// `coinbase(address)` — set block beneficiary.
pub const COINBASE_SELECTOR: [u8; 4] = [0xff, 0x48, 0x3c, 0x54];
/// `difficulty(uint256)` — no-op (post-merge).
pub const DIFFICULTY_SELECTOR: [u8; 4] = [0x46, 0xcc, 0x92, 0xd9];
/// `prevrandao(bytes32)` — set prevrandao.
pub const PREVRANDAO_SELECTOR: [u8; 4] = [0x3b, 0x92, 0x55, 0x49];
/// `chainId(uint256)` — set chain ID.
pub const CHAIN_ID_SELECTOR: [u8; 4] = [0x40, 0x49, 0xdd, 0xd2];

pub fn handle_warp<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let value = decode_u256_arg(input)?;
    inspector.state.warp_timestamp = Some(value);
    let mut block = ctx.block().clone();
    block.timestamp = value;
    ctx.set_block(block);
    Some(dummy_success())
}

pub fn handle_roll<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let value = decode_u256_arg(input)?;
    inspector.state.roll_number = Some(value);
    let mut block = ctx.block().clone();
    block.number = value;
    ctx.set_block(block);
    Some(dummy_success())
}

pub fn handle_fee<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let value = decode_u256_arg(input)?;
    inspector.state.fee = Some(value);
    let mut block = ctx.block().clone();
    block.basefee = u64::try_from(value).unwrap_or(0);
    ctx.set_block(block);
    Some(dummy_success())
}

pub fn handle_coinbase<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = super::decode_address_arg(input)?;
    inspector.state.coinbase = Some(addr);
    let mut block = ctx.block().clone();
    block.beneficiary = addr;
    ctx.set_block(block);
    Some(dummy_success())
}

pub fn handle_prevrandao<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if input.len() < 4 + 32 {
        return None;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&input[4..4 + 32]);
    inspector.state.prevrandao = Some(bytes);
    let mut block = ctx.block().clone();
    block.prevrandao = Some(revm::primitives::FixedBytes::from(bytes));
    ctx.set_block(block);
    Some(dummy_success())
}

pub fn handle_chain_id<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv> + ContextSetters>(
    inspector: &mut CheatcodeInspector,
    _ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let value = decode_u256_arg(input)?;
    inspector.state.chain_id = Some(value);
    Some(dummy_success())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::{
        MainContext,
        context::Context,
        database::InMemoryDB,
        primitives::{Address, U256},
    };

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], value: U256) -> revm::primitives::Bytes {
        let mut data = selector.to_vec();
        data.extend_from_slice(&value.to_be_bytes_vec());
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn warp_sets_timestamp() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let ts = U256::from(1234567890u64);
        let result = handle_warp(&mut inspector, &mut ctx, &call_data(WARP_SELECTOR, ts));
        assert!(result.is_some());
        assert_eq!(ctx.block.timestamp, ts);
        assert_eq!(inspector.state.warp_timestamp, Some(ts));
    }

    #[test]
    fn roll_sets_number() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let num = U256::from(42u64);
        let result = handle_roll(&mut inspector, &mut ctx, &call_data(ROLL_SELECTOR, num));
        assert!(result.is_some());
        assert_eq!(ctx.block.number, num);
    }

    #[test]
    fn fee_sets_basefee() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let fee = U256::from(10u64);
        let result = handle_fee(&mut inspector, &mut ctx, &call_data(FEE_SELECTOR, fee));
        assert!(result.is_some());
        assert_eq!(ctx.block.basefee, 10);
    }

    #[test]
    fn coinbase_sets_beneficiary() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xca; 20]);
        let mut data = vec![0u8; 4 + 32];
        data[0..4].copy_from_slice(&COINBASE_SELECTOR);
        data[4 + 12..4 + 32].copy_from_slice(addr.as_slice());
        let result = handle_coinbase(
            &mut inspector,
            &mut ctx,
            &revm::primitives::Bytes::from(data),
        );
        assert!(result.is_some());
        assert_eq!(ctx.block.beneficiary, addr);
    }

    #[test]
    fn chain_id_sets_state() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let id = U256::from(1337u64);
        let result = handle_chain_id(&mut inspector, &mut ctx, &call_data(CHAIN_ID_SELECTOR, id));
        assert!(result.is_some());
        assert_eq!(inspector.state.chain_id, Some(id));
    }

    #[test]
    fn cheatcode_state_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeState.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_selector: [u8; 4] = [0x0a, 0x7a, 0x1c, 0x4d]; // action()
        let calls = vec![Call {
            selector: action_selector,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain
            .execute_with_opts(
                &calls,
                crate::chain::executor::ExecutionOptions { trace: true },
            )
            .unwrap();
        assert!(output.all_ok, "action() should succeed");
        if let Some(ref trace) = output.trace {
            eprintln!("TRACE:\n{}", trace.format());
        }
        for p in &output.property_results {
            eprintln!("property {}: passed={}", p.name, p.passed);
        }
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "property_state_correct should pass"
        );
    }
}
