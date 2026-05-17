//! Block / state manipulation cheatcodes.

use revm::primitives::{Bytes, U256};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect, decode_u256_arg};

pub struct Prevrandao;
impl Cheatcode for Prevrandao {
    type Args = [u8; 32];
    const SELECTOR: [u8; 4] = [0x3b, 0x92, 0x55, 0x49];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 32 {
            return None;
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&input[4..4 + 32]);
        Some(bytes)
    }
    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetPrevrandao(value)]
    }
}

pub struct ChainId;
impl Cheatcode for ChainId {
    type Args = U256;
    const SELECTOR: [u8; 4] = [0x40, 0x49, 0xdd, 0xd2];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_u256_arg(input)
    }
    fn effects(value: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetChainId(value)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::U256;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn chain_id_decode_and_effects() {
        let mut data = ChainId::SELECTOR.to_vec();
        data.extend_from_slice(&U256::from(1337u64).to_be_bytes_vec());
        let args = ChainId::decode(&Bytes::from(data)).unwrap();
        assert_eq!(
            ChainId::effects(args),
            vec![CheatcodeEffect::SetChainId(U256::from(1337u64))]
        );
    }

    #[test]
    #[serial]
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
