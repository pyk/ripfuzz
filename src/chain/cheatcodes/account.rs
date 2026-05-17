//! Account manipulation cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::{
    context_interface::{ContextTr, JournalTr, journaled_state::account::JournaledAccountTr},
    database::InMemoryDB,
    interpreter::CallOutcome,
    primitives::U256,
};

use crate::chain::cheatcodes::{
    CheatcodeInspector, decode_address_arg, decode_address_u256_args, dummy_success,
    success_u256_outcome,
};

/// `deal(address, uint256)` — set balance.
pub const DEAL_SELECTOR: [u8; 4] = [0xc8, 0x8a, 0x5e, 0x6d];
/// `etch(address, bytes)` — set code.
pub const ETCH_SELECTOR: [u8; 4] = [0xb4, 0xd6, 0xc7, 0x82];
/// `setNonce(address, uint64)` — set nonce.
pub const SET_NONCE_SELECTOR: [u8; 4] = [0xf8, 0xe1, 0x8b, 0x57];
/// `getNonce(address)` returns `uint64`.
pub const GET_NONCE_SELECTOR: [u8; 4] = [0x2d, 0x03, 0x35, 0xab];
/// `load(address, bytes32)` returns `bytes32`.
pub const LOAD_SELECTOR: [u8; 4] = [0x66, 0x7f, 0x9d, 0x70];
/// `store(address, bytes32, bytes32)` — set storage.
pub const STORE_SELECTOR: [u8; 4] = [0x70, 0xca, 0x10, 0xbb];

pub fn handle_deal<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (addr, value) = decode_address_u256_args(input)?;
    let mut acc = ctx.journal_mut().load_account_mut(addr).ok()?.data;
    acc.set_balance(value);
    Some(dummy_success())
}

pub fn handle_etch<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let tuple = DynSolType::Tuple(vec![DynSolType::Address, DynSolType::Bytes]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        _ => return None,
    };
    if values.len() != 2 {
        return None;
    }
    let addr = match &values[0] {
        DynSolValue::Address(a) => *a,
        _ => return None,
    };
    let code = match &values[1] {
        DynSolValue::Bytes(b) => revm::primitives::Bytes::from(b.clone()),
        _ => return None,
    };
    let bytecode = revm::bytecode::Bytecode::new_raw(code);
    let mut acc = ctx.journal_mut().load_account_mut(addr).ok()?.data;
    acc.set_code_and_hash_slow(bytecode);
    Some(dummy_success())
}

pub fn handle_set_nonce<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = revm::primitives::Address::from_slice(&input[4 + 12..4 + 32]);
    let nonce = match u64::try_from(U256::from_be_slice(&input[4 + 32..4 + 64])) {
        Ok(v) => v,
        Err(_) => return None,
    };
    let mut acc = ctx.journal_mut().load_account_mut(addr).ok()?.data;
    acc.set_nonce(nonce);
    Some(dummy_success())
}

pub fn handle_get_nonce<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = decode_address_arg(input)?;
    let nonce = ctx
        .journal_mut()
        .load_account(addr)
        .ok()
        .map(|s| s.data.info.nonce)
        .unwrap_or(0);
    Some(success_u256_outcome(U256::from(nonce)))
}

pub fn handle_load<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (addr, slot) = super::decode_address_bytes32_args(input)?;
    let slot_u256 = U256::from_be_bytes(slot);
    let mut acc = ctx.journal_mut().load_account_mut(addr).ok()?.data;
    let value = acc.sload(slot_u256, false).ok()?.data.present_value;
    Some(super::success_bytes_outcome(value.to_be_bytes_vec()))
}

pub fn handle_store<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (addr, slot, value) = super::decode_address_bytes32_bytes32_args(input)?;
    let slot_u256 = U256::from_be_bytes(slot);
    let value_u256 = U256::from_be_bytes(value);
    let mut acc = ctx.journal_mut().load_account_mut(addr).ok()?.data;
    let _ = acc.sstore(slot_u256, value_u256, false);
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
        state::AccountInfo,
    };

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], payload: Vec<u8>) -> revm::primitives::Bytes {
        let mut data = selector.to_vec();
        data.extend_from_slice(&payload);
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn deal_sets_balance() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xab; 20]);
        let value = U256::from(5_000u64);
        let mut payload = vec![0u8; 64];
        payload[12..32].copy_from_slice(addr.as_slice());
        payload[32..64].copy_from_slice(&value.to_be_bytes_vec());
        let result = handle_deal(&mut inspector, &mut ctx, &call_data(DEAL_SELECTOR, payload));
        assert!(result.is_some());
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert_eq!(info.info.balance, value);
    }

    #[test]
    fn set_nonce_updates_nonce() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xcd; 20]);
        let nonce: u64 = 7;
        let mut payload = vec![0u8; 64];
        payload[12..32].copy_from_slice(addr.as_slice());
        payload[32..64].copy_from_slice(&U256::from(nonce).to_be_bytes_vec());
        let result = handle_set_nonce(
            &mut inspector,
            &mut ctx,
            &call_data(SET_NONCE_SELECTOR, payload),
        );
        assert!(result.is_some());
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert_eq!(info.info.nonce, nonce);
    }

    #[test]
    fn get_nonce_reads_nonce() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xef; 20]);
        let mut info = AccountInfo::default();
        info.nonce = 42;
        ctx.db_mut().insert_account_info(addr, info);
        let mut payload = vec![0u8; 32];
        payload[12..32].copy_from_slice(addr.as_slice());
        let result = handle_get_nonce(
            &mut inspector,
            &mut ctx,
            &call_data(GET_NONCE_SELECTOR, payload),
        );
        assert!(result.is_some());
        let out = result.unwrap().result.output;
        assert_eq!(U256::from_be_slice(&out), U256::from(42));
    }

    #[test]
    fn store_and_load_roundtrip() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0x11; 20]);
        let slot = [0x22u8; 32];
        let value = [0x33u8; 32];
        let mut payload = vec![0u8; 96];
        payload[12..32].copy_from_slice(addr.as_slice());
        payload[32..64].copy_from_slice(&slot);
        payload[64..96].copy_from_slice(&value);
        let store_result = handle_store(
            &mut inspector,
            &mut ctx,
            &call_data(STORE_SELECTOR, payload.clone()),
        );
        assert!(store_result.is_some());

        let mut load_payload = vec![0u8; 64];
        load_payload[12..32].copy_from_slice(addr.as_slice());
        load_payload[32..64].copy_from_slice(&slot);
        let load_result = handle_load(
            &mut inspector,
            &mut ctx,
            &call_data(LOAD_SELECTOR, load_payload),
        );
        assert!(load_result.is_some());
        assert_eq!(
            load_result.unwrap().result.output.as_ref(),
            value.as_slice()
        );
    }

    #[test]
    fn etch_sets_code() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xbe; 20]);
        let code = vec![
            0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x60, 0x00, 0xf3,
        ];

        // Manual ABI encoding for etch(address,bytes):
        // [selector][padded_addr][bytes_offset][bytes_length][bytes_data...]
        let mut data = ETCH_SELECTOR.to_vec();
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        let bytes_offset: u32 = 64; // 2 * 32
        let mut offset_word = vec![0u8; 32];
        offset_word[28..32].copy_from_slice(&bytes_offset.to_be_bytes());
        data.extend_from_slice(&offset_word);
        let mut len_word = vec![0u8; 32];
        len_word[28..32].copy_from_slice(&(code.len() as u32).to_be_bytes());
        data.extend_from_slice(&len_word);
        let mut code_padded = code.clone();
        while code_padded.len() % 32 != 0 {
            code_padded.push(0);
        }
        data.extend_from_slice(&code_padded);

        let result = handle_etch(
            &mut inspector,
            &mut ctx,
            &revm::primitives::Bytes::from(data),
        );
        assert!(result.is_some(), "etch should decode and succeed");
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert!(info.info.code.is_some());
        assert!(!info.info.code_hash.is_zero());
    }

    #[test]
    fn cheatcode_account_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAccount.sol"),
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

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "action() should succeed");
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "all account properties should pass"
        );
    }
}
