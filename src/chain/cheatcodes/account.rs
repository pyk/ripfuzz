//! Account manipulation cheatcodes.

use revm::primitives::{Address, Bytes, U256};

use crate::chain::cheatcodes::{
    Cheatcode, CheatcodeEffect, decode_address_bytes32_args, decode_address_bytes32_bytes32_args,
};

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
        database::InMemoryDB,
        primitives::{Address, U256},
    };
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

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
