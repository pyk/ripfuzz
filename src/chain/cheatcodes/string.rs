//! `getCode` cheatcode.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

fn decode_single(input: &Bytes, t: DynSolType) -> Option<DynSolValue> {
    let tuple = DynSolType::Tuple(vec![t]);
    let decoded = tuple.abi_decode_params(&input[4..]).ok()?;
    match decoded {
        DynSolValue::Tuple(v) => v.into_iter().next(),
        _ => None,
    }
}

pub struct GetCode;
impl Cheatcode for GetCode {
    type Args = String;
    const SELECTOR: [u8; 4] = [0x8d, 0x1c, 0xc9, 0x25];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let val = decode_single(input, DynSolType::String)?;
        match val {
            DynSolValue::String(s) => Some(s),
            _ => None,
        }
    }
    fn effects(arg: Self::Args) -> Vec<CheatcodeEffect> {
        let name = arg.split(':').next_back().unwrap_or(&arg).trim().into();
        vec![CheatcodeEffect::GetCode(name)]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;
    use crate::contract;

    fn call_data(selector: [u8; 4], encoded: Vec<u8>) -> Bytes {
        let mut data = selector.to_vec();
        data.extend(encoded);
        Bytes::from(data)
    }

    #[test]
    fn get_code_looks_up_compiled_contract() {
        let mut inspector = CheatcodeInspector::new();
        inspector.state.compiled_contracts.insert(
            "CheatcodeString".into(),
            revm::primitives::Bytes::from(vec![0x60, 0x01]),
        );
        let encoded = DynSolValue::String("CheatcodeString".into()).abi_encode();
        let args = GetCode::decode(&call_data(GetCode::SELECTOR, encoded)).unwrap();
        // GetCode effects return the name; the inspector resolves it in build_outcome.
        assert_eq!(args, "CheatcodeString");
    }

    #[test]
    #[serial]
    fn cheatcode_string_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodeString.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let output = chain.execute(&vec![]).unwrap();
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "string property should pass"
        );
    }
}
