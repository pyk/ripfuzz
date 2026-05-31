//! A single failed assertion discovered during fuzzing.

use std::collections::HashMap;

use revm::primitives::Bytes;

use crate::corpus::Item;
use crate::evm;
use crate::evm::Transaction;

/// A single failed assertion (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FailedAssertion {
    pub transactions: Vec<Transaction>,
    /// The corpus item that produced this failure.
    pub item: Item,
}

impl FailedAssertion {
    /// Format this failed assertion's call sequence as a flat, Medusa-style log.
    pub fn format(&self, contract: &evm::Contract) -> String {
        let mut selector_map = HashMap::new();
        for func in contract
            .target_functions
            .iter()
            .chain(contract.invariant_functions.iter())
        {
            let sel: [u8; 4] = func.selector().into();
            // checkrs: allow(clone_in_loops)
            selector_map.insert(sel, func.name.clone());
        }

        let mut lines = Vec::new();
        for (i, tx) in self.transactions.iter().enumerate() {
            let n = i + 1;
            let name = Self::format_calldata(&tx.calldata, &selector_map);
            lines.push(format!("    {n}. {name}"));
        }
        lines.join("\n")
    }

    fn format_calldata(calldata: &Bytes, selector_map: &HashMap<[u8; 4], String>) -> String {
        if calldata.len() < 4 {
            return "()".into();
        }
        let selector: [u8; 4] = calldata[0..4].try_into().unwrap_or([0; 4]);
        if let Some(name) = selector_map.get(&selector) {
            format!("{}()", name)
        } else {
            format!("0x{}", hex::encode(&calldata[0..4]))
        }
    }
}
