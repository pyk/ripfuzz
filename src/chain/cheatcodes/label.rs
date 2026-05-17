//! Label cheatcode.

#[cfg(test)]
use std::collections::HashMap;

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, dummy_success};

/// `label(address, string)`.
pub const LABEL_SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];

pub fn handle_label<CTX: revm::context_interface::ContextTr>(
    inspector: &mut CheatcodeInspector,
    _ctx: &mut CTX,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    if input.len() < 4 {
        return Some(dummy_success());
    }
    let types = vec![DynSolType::Address, DynSolType::String];
    let tuple = DynSolType::Tuple(types);
    let decoded = match tuple.abi_decode_params(&input[4..]) {
        Ok(v) => v,
        Err(_) => return Some(dummy_success()),
    };
    let values = match decoded {
        DynSolValue::Tuple(v) => v,
        _ => return Some(dummy_success()),
    };
    if values.len() != 2 {
        return Some(dummy_success());
    }
    let addr = match &values[0] {
        DynSolValue::Address(a) => *a,
        _ => return Some(dummy_success()),
    };
    let name = match &values[1] {
        DynSolValue::String(s) => s.clone(),
        _ => return Some(dummy_success()),
    };
    inspector.state.labels.insert(addr, name.clone());
    if let Some(ref labels) = inspector.shared_labels
        && let Ok(mut guard) = labels.write()
    {
        guard.insert(addr, name);
    }
    Some(dummy_success())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use alloy_dyn_abi::DynSolValue;
    use revm::{MainContext, primitives::Address};

    use super::*;
    use crate::chain::cheatcodes::CheatcodeInspector;

    /// Build `label(address, string)` calldata manually to match Solidity ABI.
    fn label_calldata(addr: Address, name: &str) -> revm::primitives::Bytes {
        let mut data = LABEL_SELECTOR.to_vec();
        // address parameter
        let mut param1 = vec![0u8; 32];
        param1[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&param1);
        // string offset (64 = 2 * 32 bytes from start of params)
        let mut param2 = vec![0u8; 32];
        param2[31] = 64;
        data.extend_from_slice(&param2);
        // string length
        let mut len = vec![0u8; 32];
        len[31] = name.len() as u8;
        data.extend_from_slice(&len);
        // string data padded to 32 bytes
        let mut str_data = vec![0u8; 32];
        str_data[..name.len()].copy_from_slice(name.as_bytes());
        data.extend_from_slice(&str_data);
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn label_inserts_into_state() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let name = "MyContract";
        let input = label_calldata(addr, name);
        // Verify the handler's decoder can parse it.
        let types = vec![DynSolType::Address, DynSolType::String];
        let tuple = DynSolType::Tuple(types);
        let decoded = tuple.abi_decode_params(&input[4..]).unwrap();
        assert_eq!(
            decoded,
            DynSolValue::Tuple(vec![
                DynSolValue::Address(addr),
                DynSolValue::String(name.into()),
            ])
        );

        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        let result = handle_label(&mut inspector, &mut ctx, &input);
        assert!(result.is_some());
        assert_eq!(inspector.state.labels.get(&addr), Some(&name.to_string()));
    }

    #[test]
    fn label_writes_to_shared_map() {
        let shared = Arc::new(RwLock::new(HashMap::new()));
        let mut inspector = CheatcodeInspector::new().with_shared_labels(Arc::clone(&shared));
        let addr = Address::new([0xcd; 20]);
        let name = "SharedLabel";
        let input = label_calldata(addr, name);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        let result = handle_label(&mut inspector, &mut ctx, &input);
        assert!(result.is_some());
        let guard = shared.read().unwrap();
        assert_eq!(guard.get(&addr), Some(&name.to_string()));
    }
}
