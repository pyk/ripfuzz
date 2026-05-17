//! Label cheatcode.

use alloy_dyn_abi::{DynSolType, DynSolValue};
use revm::primitives::{Address, Bytes};

use crate::chain::cheatcodes::{Cheatcode, CheatcodeEffect};

pub struct Label;
impl Cheatcode for Label {
    type Args = (Address, String);
    const SELECTOR: [u8; 4] = [0xc6, 0x57, 0xc7, 0x18];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 {
            return None;
        }
        let types = vec![DynSolType::Address, DynSolType::String];
        let tuple = DynSolType::Tuple(types);
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
        let name = match &values[1] {
            DynSolValue::String(s) => s.clone(),
            _ => return None,
        };
        Some((addr, name))
    }
    fn effects((addr, name): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::AddLabel(addr, name)]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, RwLock};

    use revm::{MainContext, primitives::Address};

    use super::*;
    use crate::chain::cheatcodes::effect::apply_effect;
    use crate::chain::inspectors::cheatcode::CheatcodeInspector;

    fn label_calldata(addr: Address, name: &str) -> Bytes {
        let mut data = Label::SELECTOR.to_vec();
        let mut param1 = vec![0u8; 32];
        param1[12..32].copy_from_slice(addr.as_slice());
        data.extend_from_slice(&param1);
        let mut param2 = vec![0u8; 32];
        param2[31] = 64;
        data.extend_from_slice(&param2);
        let mut len = vec![0u8; 32];
        len[31] = name.len() as u8;
        data.extend_from_slice(&len);
        let mut str_data = vec![0u8; 32];
        str_data[..name.len()].copy_from_slice(name.as_bytes());
        data.extend_from_slice(&str_data);
        Bytes::from(data)
    }

    #[test]
    fn label_inserts_into_state() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let name = "MyContract";
        let input = label_calldata(addr, name);
        let args = Label::decode(&input).unwrap();
        let effects = Label::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        assert_eq!(inspector.state.labels.get(&addr), Some(&name.to_string()));
    }

    #[test]
    fn label_writes_to_shared_map() {
        let shared = Arc::new(RwLock::new(HashMap::new()));
        let mut inspector = CheatcodeInspector::new().with_shared_labels(Arc::clone(&shared));
        let addr = Address::new([0xcd; 20]);
        let name = "SharedLabel";
        let input = label_calldata(addr, name);
        let args = Label::decode(&input).unwrap();
        let effects = Label::effects(args);
        let mut ctx =
            revm::context::Context::mainnet().with_db(revm::database::InMemoryDB::default());
        for e in &effects {
            apply_effect(e, &mut ctx, &mut inspector.state).unwrap();
        }
        // call() in the inspector syncs labels; simulate it here.
        if let Some(ref s) = inspector.shared_labels {
            if let Ok(mut guard) = s.write() {
                for (a, n) in &inspector.state.labels {
                    guard.insert(*a, n.clone());
                }
            }
        }
        let guard = shared.read().unwrap();
        assert_eq!(guard.get(&addr), Some(&name.to_string()));
    }
}
