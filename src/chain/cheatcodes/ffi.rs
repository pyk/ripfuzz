//! FFI cheatcode.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::Bytes;

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Ffi;
impl Cheatcode for Ffi {
    type Args = Vec<String>;
    const SELECTOR: [u8; 4] = [0x89, 0x16, 0x04, 0x67];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        let array_type = DynSolType::Array(Box::new(DynSolType::String));
        let decoded = array_type.abi_decode_params(&input[4..]).ok()?;
        let DynSolValue::Array(args) = decoded else {
            return None;
        };
        let mut result = Vec::new();
        for arg in args {
            if let DynSolValue::String(s) = arg {
                result.push(s);
            } else {
                return None;
            }
        }
        Some(result)
    }
    fn effects(args: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::FfiExec(args)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_decode_works() {
        let input = Bytes::from(vec![0x0a, 0x94, 0xd9, 0x2e, 0x00, 0x00, 0x00, 0x00]);
        assert!(Ffi::decode(&input).is_none());
    }
}
