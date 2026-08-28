//! Failed assertions discovered during fuzzing and the shared, deduplicated
//! collection that tracks them across fuzzer threads.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;
use revm::primitives::Bytes;

use crate::CoverageId;
use crate::corpus::Item;
use crate::evm;
use crate::evm::Transaction;

/// A single failed assertion (assert panic) discovered during fuzzing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FailedAssertion {
    pub transactions: Vec<Transaction>,
    /// The corpus item that produced this failure.
    pub item: Item,
    /// Index of the first transaction that triggered the failure.
    #[serde(default)]
    pub failure_index: Option<usize>,
    /// Contract coverage id and PC identifying the failed `assert`
    /// statement, when execution captured it.
    #[serde(default)]
    pub failure_pc: Option<(CoverageId, usize)>,
}

impl FailedAssertion {
    /// Key used to deduplicate failed assertions from the same bug.
    ///
    /// Two failures are treated as the same assertion when the first failing call
    /// is at the same position and the function signature sequence matches,
    /// regardless of the concrete argument values. When the exact assertion
    /// PC is available, it takes precedence so multiple assertions inside one
    /// function are treated as distinct bugs.
    pub fn dedup_key(&self) -> String {
        if let Some((contract, pc)) = self.failure_pc {
            return format!("pc:{}:{pc}", hex::encode(contract.codehash()));
        }
        let sequence = self
            .item
            .calls
            .iter()
            .map(|call| call.function.signature())
            .collect::<Vec<String>>()
            .join(">");
        format!("{}:{sequence}", self.failure_index.unwrap_or(0))
    }

    /// Format this failed assertion's call sequence as a flat, Medusa-style log.
    pub fn format(&self, contract: &evm::Contract) -> String {
        let mut selector_map = HashMap::new();
        for func in contract
            .handler_functions
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

#[derive(Debug)]
struct SharedFailedAssertionsInner {
    max_failures: usize,
    assertions: Vec<FailedAssertion>,
    seen: HashSet<String>,
}

/// Thread-safe collection of distinct failed assertions.
///
/// Fuzzer threads add failed assertions as they find them. Once
/// `max_failures` distinct failed assertions have been collected, the
/// collection is full and the campaign should stop scheduling new runs.
///
/// Cloning is cheap (shares the same inner state).
#[derive(Debug, Clone)]
pub struct SharedFailedAssertions {
    inner: Arc<Mutex<SharedFailedAssertionsInner>>,
}

impl SharedFailedAssertions {
    /// Create a new failed assertion collection with the given capacity.
    pub fn new(max_failures: usize) -> Self {
        assert!(max_failures > 0, "max_failures must be at least 1");
        Self {
            inner: Arc::new(Mutex::new(SharedFailedAssertionsInner {
                max_failures,
                assertions: Vec::new(),
                seen: HashSet::new(),
            })),
        }
    }

    /// Add a failed assertion if it is distinct and the collection is not full.
    ///
    /// Returns `true` when the failed assertion was newly inserted.
    pub fn try_add(&self, failure: FailedAssertion) -> bool {
        let mut inner = self.inner.lock();
        if inner.assertions.len() >= inner.max_failures {
            return false;
        }
        if inner.seen.insert(failure.dedup_key()) {
            inner.assertions.push(failure);
            true
        } else {
            false
        }
    }

    /// Number of distinct failed assertions currently collected.
    pub fn len(&self) -> usize {
        self.inner.lock().assertions.len()
    }

    /// Whether no failed assertions have been collected yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the collection reached its configured capacity.
    pub fn is_full(&self) -> bool {
        let inner = self.inner.lock();
        inner.assertions.len() >= inner.max_failures
    }

    /// Clone the collected failed assertions in insertion order.
    pub fn items(&self) -> Vec<FailedAssertion> {
        self.inner.lock().assertions.clone()
    }
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, B256, U256};

    use crate::CoverageId;

    use super::*;
    use crate::corpus::{Call, Item};

    fn call(signature: &str, value: u64) -> Call {
        Call {
            function: Function::parse(signature).unwrap(),
            args: DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(value), 256)]),
            ..Default::default()
        }
    }

    fn failure(signature: &str, value: u64, failure_index: Option<usize>) -> FailedAssertion {
        FailedAssertion {
            transactions: Vec::new(),
            item: Item::from(vec![call(signature, value)]),
            failure_index,
            failure_pc: None,
        }
    }

    #[test]
    fn exact_duplicate_is_deduplicated() {
        let assertions = SharedFailedAssertions::new(8);

        assert!(assertions.try_add(failure("f(uint256)", 1, Some(0))));
        assert!(!assertions.try_add(failure("f(uint256)", 1, Some(0))));
        assert_eq!(assertions.len(), 1);
    }

    #[test]
    fn same_bug_with_different_args_is_deduplicated() {
        let assertions = SharedFailedAssertions::new(8);

        assert!(assertions.try_add(failure("f(uint256)", 1, Some(0))));
        assert!(!assertions.try_add(failure("f(uint256)", 2, Some(0))));
        assert_eq!(assertions.len(), 1);
    }

    #[test]
    fn different_failure_positions_are_distinct_assertions() {
        let assertions = SharedFailedAssertions::new(8);

        let mut first = failure("f(uint256)", 1, Some(0));
        first.item.calls.push(call("g(uint256)", 2));
        let mut second = failure("f(uint256)", 1, Some(0));
        second.item.calls.push(call("g(uint256)", 2));
        second.failure_index = Some(1);

        assert!(assertions.try_add(first));
        assert!(assertions.try_add(second));
        assert_eq!(assertions.len(), 2);
    }

    #[test]
    fn same_assertion_pc_deduplicates_across_sequences() {
        let assertions = SharedFailedAssertions::new(8);

        let mut first = failure("f(uint256)", 1, Some(0));
        first.failure_pc = Some((CoverageId::Initcode(B256::from([0xab; 32])), 10));
        let mut second = failure("g(uint256)", 2, Some(1));
        second.failure_pc = Some((CoverageId::Initcode(B256::from([0xab; 32])), 10));

        assert!(assertions.try_add(first));
        assert!(!assertions.try_add(second));
        assert_eq!(assertions.len(), 1);
    }

    #[test]
    fn different_assertion_pcs_in_same_function_are_distinct() {
        let assertions = SharedFailedAssertions::new(8);

        let mut first = failure("f(uint256)", 1, Some(0));
        first.failure_pc = Some((CoverageId::Initcode(B256::from([0xab; 32])), 10));
        let mut second = failure("f(uint256)", 1, Some(0));
        second.failure_pc = Some((CoverageId::Initcode(B256::from([0xab; 32])), 20));

        assert!(assertions.try_add(first));
        assert!(assertions.try_add(second));
        assert_eq!(assertions.len(), 2);
    }

    #[test]
    fn different_sequences_are_distinct_assertions() {
        let assertions = SharedFailedAssertions::new(8);

        let mut first = failure("f(uint256)", 1, Some(0));
        first.item.calls.push(call("g(uint256)", 2));
        let second = failure("h(uint256)", 1, Some(0));

        assert!(assertions.try_add(first));
        assert!(assertions.try_add(second));
        assert_eq!(assertions.len(), 2);
    }

    #[test]
    fn capacity_stops_collection() {
        let assertions = SharedFailedAssertions::new(2);

        assert!(assertions.try_add(failure("f(uint256)", 1, Some(0))));
        assert!(assertions.try_add(failure("g(uint256)", 1, Some(0))));
        assert!(!assertions.try_add(failure("h(uint256)", 1, Some(0))));
        assert!(assertions.is_full());
        assert_eq!(assertions.len(), 2);
    }
}
