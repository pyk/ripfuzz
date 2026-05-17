//! Account manipulation cheatcodes.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::{Address, Bytes, U256};

use crate::chain::cheatcodes::{
    Cheatcode, CheatcodeEffect, decode_address_arg, decode_address_bytes32_args,
    decode_address_bytes32_bytes32_args,
};

pub struct Etch;
impl Cheatcode for Etch {
    type Args = (Address, Bytes);
    const SELECTOR: [u8; 4] = [0xb4, 0xd6, 0xc7, 0x82];
    fn decode(input: &Bytes) -> Option<Self::Args> {
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
            DynSolValue::Bytes(b) => Bytes::from(b.clone()),
            _ => return None,
        };
        Some((addr, code))
    }
    fn effects((addr, code): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetAccountCode(addr, code)]
    }
}

pub struct SetNonce;
impl Cheatcode for SetNonce {
    type Args = (Address, u64);
    const SELECTOR: [u8; 4] = [0xf8, 0xe1, 0x8b, 0x57];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 64 {
            return None;
        }
        let addr = Address::from_slice(&input[4 + 12..4 + 32]);
        let nonce = u64::try_from(U256::from_be_slice(&input[4 + 32..4 + 64])).ok()?;
        Some((addr, nonce))
    }
    fn effects((addr, nonce): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetAccountNonce(addr, nonce)]
    }
}

pub struct GetNonce;
impl Cheatcode for GetNonce {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x2d, 0x03, 0x35, 0xab];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }
    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::ReadNonce(addr)]
    }
}

pub struct Load;
impl Cheatcode for Load {
    type Args = (Address, [u8; 32]);
    const SELECTOR: [u8; 4] = [0x66, 0x7f, 0x9d, 0x70];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_bytes32_args(input)
    }
    fn effects((addr, slot): Self::Args) -> Vec<CheatcodeEffect> {
        let slot_u256 = U256::from_be_bytes(slot);
        vec![CheatcodeEffect::ReadStorage(addr, slot_u256)]
    }
}

pub struct Store;
impl Cheatcode for Store {
    type Args = (Address, [u8; 32], [u8; 32]);
    const SELECTOR: [u8; 4] = [0x70, 0xca, 0x10, 0xbb];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_bytes32_bytes32_args(input)
    }
    fn effects((addr, slot, value): Self::Args) -> Vec<CheatcodeEffect> {
        let slot_u256 = U256::from_be_bytes(slot);
        let value_u256 = U256::from_be_bytes(value);
        vec![CheatcodeEffect::SetStorage(addr, slot_u256, value_u256)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::{
        MainContext,
        context::Context,
        context_interface::{ContextTr, JournalTr},
        database::InMemoryDB,
        primitives::{Address, U256},
        state::AccountInfo,
    };
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn set_nonce_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xcd; 20]);
        let nonce: u64 = 7;
        let effects = SetNonce::effects((addr, nonce));
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert_eq!(info.info.nonce, nonce);
    }

    #[test]
    fn get_nonce_read_effect() {
        let _inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xef; 20]);
        let mut info = AccountInfo::default();
        info.nonce = 42;
        ctx.db_mut().insert_account_info(addr, info);
        let effects = GetNonce::effects(addr);
        assert_eq!(effects, vec![CheatcodeEffect::ReadNonce(addr)]);
    }

    #[test]
    fn store_and_load_roundtrip() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0x11; 20]);
        let slot = [0x22u8; 32];
        let value = [0x33u8; 32];
        let store_effects = Store::effects((addr, slot, value));
        for e in &store_effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }

        let load_effects = Load::effects((addr, slot));
        assert_eq!(
            load_effects,
            vec![CheatcodeEffect::ReadStorage(
                addr,
                U256::from_be_bytes(slot)
            )]
        );
    }

    #[test]
    fn etch_effect_applies() {
        let mut inspector = CheatcodeInspector::new();
        let mut ctx = Context::mainnet().with_db(InMemoryDB::default());
        let addr = Address::new([0xbe; 20]);
        let code = vec![
            0x60, 0x01, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0x60, 0x00, 0xf3,
        ];

        // Manual ABI encoding for etch(address,bytes):
        let mut data = Etch::SELECTOR.to_vec();
        let mut padded_addr = vec![0u8; 32];
        padded_addr[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&padded_addr);
        let bytes_offset: u32 = 64;
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

        let args = Etch::decode(&Bytes::from(data)).unwrap();
        let effects = Etch::effects(args);
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        let info = ctx.journal_mut().load_account(addr).unwrap().data;
        assert!(info.info.code.is_some());
        assert!(!info.info.code_hash.is_zero());
    }

    #[test]
    #[serial]
    fn cheatcode_account_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeAccount.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let call_selector: [u8; 4] = [0x28, 0xb5, 0xe3, 0x2b]; // call()
        let calls = vec![Call {
            selector: call_selector,
            args: vec![],
            block_number_delay: 0,
            block_timestamp_delay: 0,
            ..Default::default()
        }];

        let output = chain.execute(&calls).unwrap();
        assert!(output.all_ok, "call() should succeed");
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "all account properties should pass"
        );
    }
}
