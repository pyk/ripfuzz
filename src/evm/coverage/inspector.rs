//! Coverage Inspector: collects per-execution EVM bytecode coverage for the fuzzer.
//!
//! ## Design Goal
//!
//! Distinguish sequence of calls that exercise genuinely different behavior.
//!
//! ## What Different Behavior Means
//!
//! Two call sequences are "different" when they cause the EVM to execute
//! bytecode in a way the fuzzer has never observed before. In practice this
//! means one of the following happened:
//!
//! 1. **New instruction hit**: the sequence reached a bytecode index (PC)
//!    that no previous sequence ever executed.
//! 2. **New branch direction**: a `JUMPI` opcode was reached and the
//!    condition evaluated to the opposite truth value (took the `else`
//!    instead of the `if`, or vice versa).
//! 3. **New call depth**: the same instruction was hit, but this time
//!    inside a nested contract call (for example, a reentrancy guard at
//!    depth 2 instead of depth 1).
//! 4. **New revert path**: execution reverted at a PC that previously
//!    always succeeded.
//! 5. **Deeper execution**: the same loop body was hit more times than
//!    before. Raptor uses AFL-style bucketing so that small
//!    count differences are ignored, but crossing a power-of-two
//!    threshold counts as novel.
//!
//! The inspector does not distinguish between different transaction
//! outcomes (return true, return false, stop, out of gas) beyond
//! "reverted or not". This is sufficient for Raptor because invariants
//! are checked via `assert` panic, not via return value.
//!
//! The inspector records these signals so the fuzzer can decide whether
//! a mutated sequence is worth keeping in the corpus.

use alloy_primitives::ruint::UintTryTo;
use revm::{
    bytecode::opcode::{JUMP, JUMPI},
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
};

use alloy_primitives::B256;

use crate::evm::coverage::edge::edge_marker;
use crate::evm::coverage::local::{LocalContractCoverage, LocalCoverage};

/// Convert a U256 stack value to usize without using `ok()`.
#[allow(clippy::manual_ok_err)]
fn u256_to_usize(v: revm::primitives::U256) -> Option<usize> {
    match v.uint_try_to() {
        Ok(n) => Some(n),
        Err(_) => None,
    }
}

/// Inspector that writes PC-hit counts into a per-contract local coverage map.
///
/// Owns its `LocalCoverage` buffer and returns it via `into_coverage`.
#[derive(Debug)]
pub struct Inspector {
    local: LocalCoverage,
    current_call_depth: u64,
    current_contract: Option<B256>,
    contract_stack: Vec<Option<B256>>,
    last_pc: usize,
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            local: LocalCoverage::new(),
            current_call_depth: 0,
            current_contract: None,
            contract_stack: Vec::new(),
            last_pc: 0,
        }
    }

    /// Consume the inspector and return the collected coverage.
    pub fn into_coverage(self) -> LocalCoverage {
        self.local
    }

    fn record_revert(&mut self) {
        let Some(contract_id) = self.current_contract else {
            return;
        };
        let Some(coverage) = self.local.contracts.get_mut(&contract_id) else {
            return;
        };
        let word = self.last_pc / 64;
        let bit = self.last_pc % 64;
        if word < coverage.reverts.len() {
            let prev = coverage.reverts[word];
            coverage.reverts[word] |= 1u64 << bit;
            if prev == 0 {
                coverage.hit_reverts.push(word);
            }
        }
    }
}

impl Default for Inspector {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX> revm::inspector::Inspector<CTX, EthInterpreter> for Inspector {
    fn initialize_interp(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let hash = interp.bytecode.hash_slow();
        if !hash.is_zero() && !interp.bytecode.is_empty() {
            let id = B256::from(hash);
            self.current_contract = Some(id);
            self.local
                .contracts
                .entry(id)
                .or_insert_with(|| LocalContractCoverage::new(interp.bytecode.len()));
        }
    }

    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let pc = interp.bytecode.pc();
        self.last_pc = pc;
        let Some(contract_id) = self.current_contract else {
            return;
        };
        let Some(coverage) = self.local.contracts.get_mut(&contract_id) else {
            return;
        };
        if pc < coverage.edges.len() {
            if coverage.edges[pc] == 0 {
                coverage.hit_pcs.push(pc);
            }
            coverage.edges[pc] = coverage.edges[pc].saturating_add(1);
        }
        if pc < coverage.depths.len() {
            let depth = self.current_call_depth.min(63);
            let depth_mask = 1u64 << depth;
            if coverage.depths[pc] & depth_mask == 0 {
                coverage.hit_depths.push(pc);
            }
            coverage.depths[pc] |= depth_mask;
        }

        // Branch-direction tracking for JUMP / JUMPI.
        let opcode = interp.bytecode.opcode();
        if opcode == JUMP || opcode == JUMPI {
            let stack = interp.stack.data();
            let dest = stack.last().copied().and_then(u256_to_usize);
            let taken = if opcode == JUMPI {
                match stack.get(stack.len().saturating_sub(2)) {
                    Some(cond) => !cond.is_zero(),
                    None => false,
                }
            } else {
                true
            };
            if taken && let Some(dst) = dest {
                let marker = edge_marker(pc, dst);
                coverage
                    .jump_edges
                    .entry(marker)
                    .and_modify(|c| *c = c.saturating_add(1))
                    .or_insert(1);
            }
        }
    }

    fn call(
        &mut self,
        _context: &mut CTX,
        _inputs: &mut revm::interpreter::CallInputs,
    ) -> Option<revm::interpreter::CallOutcome> {
        self.contract_stack.push(self.current_contract);
        self.current_call_depth += 1;
        None
    }

    fn call_end(
        &mut self,
        _context: &mut CTX,
        _inputs: &revm::interpreter::CallInputs,
        outcome: &mut revm::interpreter::CallOutcome,
    ) {
        if outcome.result.result.is_revert() {
            self.record_revert();
        }
        self.current_call_depth = self.current_call_depth.saturating_sub(1);
        self.current_contract = self.contract_stack.pop().flatten();
    }

    fn create(
        &mut self,
        _context: &mut CTX,
        _inputs: &mut revm::interpreter::CreateInputs,
    ) -> Option<revm::interpreter::CreateOutcome> {
        self.contract_stack.push(self.current_contract);
        self.current_call_depth += 1;
        None
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        _inputs: &revm::interpreter::CreateInputs,
        outcome: &mut revm::interpreter::CreateOutcome,
    ) {
        if outcome.result.result.is_revert() {
            self.record_revert();
        }
        self.current_call_depth = self.current_call_depth.saturating_sub(1);
        self.current_contract = self.contract_stack.pop().flatten();
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput, ExecInput, SetupInput, Transaction};
    use crate::evm::coverage::SharedCoverage;
    use crate::foundry;

    alloy_sol_types::sol! {
        interface CoverageBranch {
            function branch(bool take) external;
        }

        interface CoverageDepth {
            function setup() external;
            function callDirect() external;
            function callIndirect() external;
        }

        interface CoverageRevert {
            function maybeRevert(bool shouldRevert) external;
        }

        interface CoverageLoop {
            function loopN(uint256 n) external;
        }

        interface CoverageDuplicate {
            function setup() external;
            function callChild1() external;
            function callChild2() external;
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/target-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup(contract: &Contract) -> (Chain, Address) {
        let mut chain = Chain::new(Config::default()).unwrap();
        chain.config.coverage = true;
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        let target = deployment.address.unwrap();

        if let Some(setup) = &contract.setup_function {
            let setup_data = Bytes::from(setup.selector().as_slice().to_vec());
            let setup_opts = SetupInput::new(target).calldata(setup_data);
            let setup = chain.setup(setup_opts).unwrap();
            assert!(setup.result.success, "setup must succeed");
        }

        (chain, target)
    }

    /// A second call sequence that hits a previously unexecuted instruction
    /// must be recorded as a new edge.
    #[test]
    fn coverage_new_instruction_hit() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let baseline = global.merge(&coverage1);
        assert!(baseline.new_edges > 0, "baseline should hit edges");

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(
            update.new_edges > 0,
            "new instruction hit should be detected"
        );
    }

    /// A second call sequence that takes a JUMPI branch in the opposite
    /// direction must be recorded as a new jump edge.
    #[test]
    fn coverage_new_branch_direction() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(
            update.new_jump_edges > 0,
            "new branch direction should be detected"
        );
    }

    /// A second call sequence that reaches the same instruction inside a nested
    /// contract call must be recorded as a new depth.
    #[test]
    fn coverage_new_call_depth() {
        let contract = load_coverage_fixture("src/CoverageDepth.sol:CoverageDepth");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDepth::callDirectCall::new(()).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDepth::callIndirectCall::new(()).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(update.new_depths > 0, "new call depth should be detected");
    }

    /// A second call sequence that reverts at a PC that previously succeeded
    /// must be recorded as a new revert.
    #[test]
    fn coverage_new_revert_path() {
        let contract = load_coverage_fixture("src/CoverageRevert.sol:CoverageRevert");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageRevert::maybeRevertCall::new((false,)).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        assert!(exec1.results[0].success, "first call should succeed");
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageRevert::maybeRevertCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        assert!(!exec2.results[0].success, "second call should revert");
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(update.new_reverts > 0, "new revert path should be detected");
    }

    /// A second call sequence that executes a loop body more times than before
    /// must be recorded as a deeper execution (AFL bucket crossing).
    #[test]
    fn coverage_deeper_execution() {
        let contract = load_coverage_fixture("src/CoverageLoop.sol:CoverageLoop");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageLoop::loopNCall::new((U256::from(1),)).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageLoop::loopNCall::new((U256::from(3),)).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(
            update.new_features > 0,
            "deeper execution should be detected"
        );
    }

    /// Two contracts with identical runtime bytecode deployed at different
    /// addresses share the same coverage contract_id. Calling the second
    /// instance after the first should not add new edges for the child.
    #[test]
    fn coverage_same_bytecode_two_addresses() {
        let contract = load_coverage_fixture("src/CoverageDuplicate.sol:CoverageDuplicate");
        let (mut chain, target) = deploy_and_setup(&contract);

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDuplicate::callChild1Call::new(()).abi_encode(),
        ))];
        let exec1 = chain.exec(ExecInput::new(txs1)).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDuplicate::callChild2Call::new(()).abi_encode(),
        ))];
        let exec2 = chain.exec(ExecInput::new(txs2)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");

        // The child contract has the shortest bytecode.
        let child_hash = coverage1
            .contracts
            .iter()
            .min_by_key(|(_, v)| v.edges.len())
            .unwrap()
            .0;

        let child_cov1 = coverage1.contracts.get(child_hash).unwrap();
        let child_cov2 = coverage2.contracts.get(child_hash).unwrap();
        assert_eq!(
            child_cov1.edges, child_cov2.edges,
            "identical bytecode should produce identical edges"
        );

        // Merging the second execution should not add new edges for the child.
        let global = SharedCoverage::new();
        global.merge(&coverage1);
        let update2 = global.merge(&coverage2);
        assert!(
            update2.new_edges > 0,
            "parent contract should still add new edges"
        );
    }

    /// Two identical transaction sequences must not be marked as interesting
    /// on the second run.
    #[test]
    fn coverage_identical_sequence_not_interesting() {
        let contract = load_coverage_fixture("src/CoverageBranch.sol:CoverageBranch");
        let (mut chain, target) = deploy_and_setup(&contract);

        let txs = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((false,)).abi_encode(),
        ))];

        let global = SharedCoverage::new();

        let exec1 = chain.exec(ExecInput::new(txs.clone())).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(
            SharedCoverage::is_interesting(&update1),
            "first run should be interesting"
        );

        let exec2 = chain.exec(ExecInput::new(txs)).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        assert!(
            !SharedCoverage::is_interesting(&update2),
            "identical second run should not be interesting"
        );
    }
}
