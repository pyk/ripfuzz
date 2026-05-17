//! Account manipulation cheatcodes.

use revm::{
    Database, context_interface::ContextTr, database::InMemoryDB, interpreter::CallOutcome,
    primitives::U256,
};

use crate::chain::cheatcodes::{
    CheatcodeInspector, decode_address_arg, decode_address_u256_args, dummy_success,
    success_u256_outcome,
};

/// `deal(address, uint256)` — set balance.
pub const DEAL_SELECTOR: [u8; 4] = [0x14, 0x07, 0xc3, 0x7c];
/// `etch(address, bytes)` — set code.
pub const ETCH_SELECTOR: [u8; 4] = [0xb5, 0xd8, 0x8c, 0x03];
/// `setNonce(address, uint64)` — set nonce.
pub const SET_NONCE_SELECTOR: [u8; 4] = [0x17, 0x74, 0xd3, 0xb5];
/// `getNonce(address)` returns `uint64`.
pub const GET_NONCE_SELECTOR: [u8; 4] = [0x2f, 0x39, 0x1c, 0x2c];
/// `load(address, bytes32)` returns `bytes32`.
pub const LOAD_SELECTOR: [u8; 4] = [0x4d, 0x23, 0x01, 0xcc];
/// `store(address, bytes32, bytes32)` — set storage.
pub const STORE_SELECTOR: [u8; 4] = [0x52, 0xef, 0x6b, 0x2c];

pub fn handle_deal<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (addr, value) = decode_address_u256_args(input)?;
    let mut info = ctx
        .db_mut()
        .basic(addr)
        .unwrap_or_default()
        .unwrap_or_default();
    info.balance = value;
    ctx.db_mut().insert_account_info(addr, info);
    Some(dummy_success())
}

pub fn handle_etch<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = revm::primitives::Address::from_slice(&input[4 + 12..4 + 32]);
    // Decode dynamic bytes: offset at 4+32, length at that offset, data follows.
    let offset = match usize::try_from(U256::from_be_slice(&input[4 + 32..4 + 64])) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if input.len() < 4 + 64 + offset + 32 {
        return None;
    }
    let data_start = 4 + 64 + offset;
    let len = match usize::try_from(U256::from_be_slice(&input[data_start..data_start + 32])) {
        Ok(v) => v,
        Err(_) => return None,
    };
    if input.len() < data_start + 32 + len {
        return None;
    }
    let code =
        revm::primitives::Bytes::copy_from_slice(&input[data_start + 32..data_start + 32 + len]);
    let mut info = ctx
        .db_mut()
        .basic(addr)
        .unwrap_or_default()
        .unwrap_or_default();
    let bytecode = revm::bytecode::Bytecode::new_raw(code);
    info.code_hash = bytecode.hash_slow();
    info.code = Some(bytecode);
    ctx.db_mut().insert_account_info(addr, info);
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
    let mut info = ctx
        .db_mut()
        .basic(addr)
        .unwrap_or_default()
        .unwrap_or_default();
    info.nonce = nonce;
    ctx.db_mut().insert_account_info(addr, info);
    Some(dummy_success())
}

pub fn handle_get_nonce<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = decode_address_arg(input)?;
    let nonce = ctx
        .db_mut()
        .basic(addr)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    Some(success_u256_outcome(U256::from(nonce)))
}

pub fn handle_load<CTX: ContextTr<Db = InMemoryDB>>(
    _inspector: &mut CheatcodeInspector,
    ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let (addr, slot) = super::decode_address_bytes32_args(input)?;
    let slot_u256 = U256::from_be_bytes(slot);
    let value = ctx.db_mut().storage(addr, slot_u256).unwrap_or(U256::ZERO);
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
    let _ = ctx
        .db_mut()
        .insert_account_storage(addr, slot_u256, value_u256);
    Some(dummy_success())
}

#[cfg(test)]
mod tests {
    use revm::{
        Database, MainContext,
        context::Context,
        database::InMemoryDB,
        primitives::{Address, U256},
        state::AccountInfo,
    };

    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

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
        let info = ctx.db_mut().basic(addr).unwrap().unwrap();
        assert_eq!(info.balance, value);
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
        let info = ctx.db_mut().basic(addr).unwrap().unwrap();
        assert_eq!(info.nonce, nonce);
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
}
