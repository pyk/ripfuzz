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
//!
//! Coverage novelty is binary: hitting the same PC or jump edge again does
//! not count as new, even if the hit count is higher.
//!
//! The inspector does not distinguish between different transaction
//! outcomes (return true, return false, stop, out of gas) beyond
//! "reverted or not". This is sufficient for Ripfuzz because invariants
//! are checked via `assert` panic, not via return value.
//!
//! The inspector records these signals so the fuzzer can decide whether
//! a mutated sequence is worth keeping in the corpus.

use alloy_primitives::ruint::UintTryTo;
use revm::{
    bytecode::opcode::{INVALID, JUMP, JUMPI, REVERT},
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
};

use alloy_primitives::B256;
use revm::interpreter::interpreter_types::InputsTr;

use crate::evm::coverage::edge::{call_edge_marker, edge_marker};
use crate::evm::coverage::exec::{ExecutionContractCoverage, ExecutionCoverage};
use crate::evm::coverage::id::CoverageId;

/// Solidity `Panic(uint256)` selector: keccak256("Panic(uint256)")[:4]
const PANIC_SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];

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
/// Owns its `ExecutionCoverage` buffer and returns it via `into_coverage`.
#[derive(Debug)]
pub struct Inspector {
    local: ExecutionCoverage,
    current_call_depth: u64,
    current_contract: Option<CoverageId>,
    contract_stack: Vec<Option<CoverageId>>,
    last_pc: usize,
    last_taken_jump_pc: Option<usize>,
    is_initcode: bool,
}

impl Inspector {
    pub fn new() -> Self {
        Self {
            local: ExecutionCoverage::new(),
            current_call_depth: 0,
            current_contract: None,
            contract_stack: Vec::new(),
            last_pc: 0,
            last_taken_jump_pc: None,
            is_initcode: false,
        }
    }

    /// Consume the inspector and return the collected coverage.
    pub fn into_coverage(self) -> ExecutionCoverage {
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
            let is_initcode = self.is_initcode;
            self.is_initcode = false;
            let id = if is_initcode {
                CoverageId::Initcode(B256::from(hash))
            } else {
                let address = interp.input.target_address();
                CoverageId::Runtime {
                    address,
                    codehash: B256::from(hash),
                }
            };
            self.current_contract = Some(id);
            self.last_taken_jump_pc = None;
            self.local.contracts.entry(id).or_insert_with(|| {
                let mut coverage = ExecutionContractCoverage::new(interp.bytecode.len());
                coverage.bytecode = interp.bytecode.original_bytes().to_vec();
                coverage.is_initcode = is_initcode;
                coverage
            });
        }
    }

    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let pc = interp.bytecode.pc();
        self.last_pc = pc;
        let Some(contract_id) = self.current_contract else {
            return;
        };
        if interp.bytecode.opcode() == INVALID {
            self.local
                .panic_pcs
                .push((contract_id, self.last_taken_jump_pc.unwrap_or(pc)));
        }
        if interp.bytecode.opcode() == REVERT {
            let stack = interp.stack.data();
            let len = stack.len();
            if len >= 2 {
                let offset = stack[len - 1];
                let size = stack[len - 2];
                if let (Some(offset), Some(size)) = (u256_to_usize(offset), u256_to_usize(size))
                    && size == 36
                {
                    let data = interp.memory.slice_len(offset, 36);
                    if data.len() >= 36 && data[..4] == PANIC_SELECTOR && data[35] == 0x01 {
                        self.local
                            .panic_pcs
                            .push((contract_id, self.last_taken_jump_pc.unwrap_or(pc)));
                    }
                }
            }
        }
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
                self.last_taken_jump_pc = Some(pc);
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
        inputs: &mut revm::interpreter::CallInputs,
    ) -> Option<revm::interpreter::CallOutcome> {
        if let Some(contract_id) = self.current_contract
            && let Some(coverage) = self.local.contracts.get_mut(&contract_id)
        {
            let marker = call_edge_marker(self.last_pc, inputs.target_address);
            coverage
                .jump_edges
                .entry(marker)
                .and_modify(|c| *c = c.saturating_add(1))
                .or_insert(1);
        }
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
        self.is_initcode = true;
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
        self.is_initcode = false;
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, U256};
    use alloy_sol_types::SolCall;
    use revm::primitives::Bytes;

    use crate::CoverageUpdate;
    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput, SetupInput, Transaction};
    use crate::evm::coverage::SharedCoverage;
    use crate::evm::coverage::edge::call_edge_marker;
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
            function callChild(uint256) external;
        }

        interface CoverageDeploy {
            function deployChild() external;
        }

        interface CoverageInitcodeFactory {
            function createChild(uint256 x) external;
        }

        interface CoverageSkip {
            function skipOrWork(uint256 x) external returns (uint256);
        }
    }

    fn load_coverage_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/harness-contract-coverage");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        Contract::try_get(&artifacts, &artifact_id).unwrap()
    }

    fn deploy_and_setup(contract: &Contract) -> (Chain, Address) {
        let config = ChainConfig::default().coverage(true);
        let mut chain = Chain::new(config).unwrap();
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
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let baseline = global.merge(&coverage1);
        assert!(baseline.new_edges > 0, "baseline should hit edges");

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
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
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageBranch::branchCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
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
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDepth::callIndirectCall::new(()).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
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
        let exec1 = chain.exec(&txs1).unwrap();
        assert!(exec1.results[0].success, "first call should succeed");
        let coverage1 = exec1.coverage.expect("coverage must be present");
        global.merge(&coverage1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageRevert::maybeRevertCall::new((true,)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        assert!(!exec2.results[0].success, "second call should revert");
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update = global.merge(&coverage2);
        assert!(update.new_reverts > 0, "new revert path should be detected");
    }

    /// A second call sequence that executes a loop body more times than before
    /// must not produce new coverage under binary edge tracking.
    #[test]
    fn coverage_deeper_execution_not_interesting() {
        let contract = load_coverage_fixture("src/CoverageLoop.sol:CoverageLoop");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageLoop::loopNCall::new((U256::from(1),)).abi_encode(),
        ))];
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(
            update1.is_interesting(),
            "first execution should be interesting (new edges)"
        );

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageLoop::loopNCall::new((U256::from(3),)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        assert!(
            !update2.is_interesting(),
            "repeated execution of the same path should not be interesting under binary coverage"
        );
    }

    /// Two clones at different addresses have distinct coverage ids even
    /// when called via the same parent function with different indices.
    /// `children[idx].doSomething()` with `idx=0` vs `idx=1` must be
    /// interesting on the second call solely due to the child address,
    /// mirroring `pools[poolId].swap` where `poolId` is otherwise silent.
    #[test]
    fn coverage_same_bytecode_two_addresses() {
        let contract = load_coverage_fixture("src/CoverageDuplicate.sol:CoverageDuplicate");
        let (mut chain, target) = deploy_and_setup(&contract);

        // Same parent function, different child index -> same parent PCs.
        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDuplicate::callChildCall::new((U256::from(0),)).abi_encode(),
        ))];
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDuplicate::callChildCall::new((U256::from(1),)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");

        // Pin child by address, not bytecode length.
        let child_id1 = *coverage1
            .contracts
            .keys()
            .find(|id| id.address() != Some(target))
            .expect("coverage must contain child contract");
        let child_id2 = *coverage2
            .contracts
            .keys()
            .find(|id| id.address() != Some(target))
            .expect("coverage must contain child contract");
        assert_ne!(
            child_id1, child_id2,
            "child contracts at different addresses should have distinct coverage ids"
        );
        assert_eq!(
            child_id1.codehash(),
            child_id2.codehash(),
            "identical bytecode should have same codehash"
        );
        let child_cov1 = coverage1.contracts.get(&child_id1).unwrap();
        let child_cov2 = coverage2.contracts.get(&child_id2).unwrap();
        assert_eq!(
            child_cov1.edges, child_cov2.edges,
            "identical bytecode should produce identical edges"
        );

        // Second call via same parent function must still be interesting
        // solely because the child address differs.
        let global = SharedCoverage::new();
        let update1 = global.merge(&coverage1);
        assert!(
            update1.is_interesting(),
            "first indexed call should be interesting"
        );
        let count_after_1 = global.contract_count();
        let update2 = global.merge(&coverage2);
        assert!(
            update2.is_interesting(),
            "second indexed call with different child via same parent function should be interesting via per-address child"
        );
        assert!(
            update2.new_edges > 0,
            "second clone must add new edges for child"
        );
        assert!(
            update2.new_jump_edges > 0,
            "second clone must add new call edge (caller_pc, callee) for caller"
        );
        let count_after_2 = global.contract_count();
        assert_eq!(
            count_after_2,
            count_after_1 + 1,
            "second child at new address should increase contract count"
        );
        // Caller-side jump edge must be distinct per callee and must equal
        // call_edge_marker(pc, callee) for some pc in the parent bytecode.
        let parent_cov1 = coverage1
            .contracts
            .iter()
            .find(|(id, _)| id.address() == Some(target))
            .map(|(_, c)| c)
            .expect("coverage must contain parent contract");
        let parent_cov2 = coverage2
            .contracts
            .iter()
            .find(|(id, _)| id.address() == Some(target))
            .map(|(_, c)| c)
            .expect("coverage must contain parent contract");
        assert!(
            !parent_cov1.jump_edges.is_empty(),
            "parent should have at least one call edge"
        );
        assert!(
            !parent_cov2.jump_edges.is_empty(),
            "parent should have at least one call edge"
        );
        assert_ne!(
            parent_cov1.jump_edges, parent_cov2.jump_edges,
            "call edge marker must differ for different callee addresses"
        );
        let child_addr1 = child_id1.address().expect("child must have address");
        let child_addr2 = child_id2.address().expect("child must have address");
        let bytecode_len = parent_cov1.bytecode.len().max(parent_cov2.bytecode.len());
        let mut found_pc: Option<usize> = None;
        for pc in 0..bytecode_len {
            let marker = call_edge_marker(pc, child_addr2);
            if parent_cov2.jump_edges.contains_key(&marker)
                && !parent_cov1.jump_edges.contains_key(&marker)
            {
                found_pc = Some(pc);
                break;
            }
        }
        assert!(
            found_pc.is_some(),
            "parent coverage for second child must contain call_edge_marker(pc, child2) for some pc in parent bytecode"
        );
        let pc = found_pc.unwrap();
        let marker1 = call_edge_marker(pc, child_addr1);
        assert!(
            parent_cov1.jump_edges.contains_key(&marker1),
            "parent coverage for first child must contain call_edge_marker(pc, child1)"
        );
        assert!(
            !parent_cov2.jump_edges.contains_key(&marker1),
            "second parent coverage must not contain marker for first child"
        );
        // Identical second call with same idx must not be interesting.
        let exec3 = chain.exec(&txs2).unwrap();
        let coverage3 = exec3.coverage.expect("coverage must be present");
        let update3 = global.merge(&coverage3);
        assert!(
            !update3.is_interesting(),
            "repeating same idx should not be interesting"
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

        let exec1 = chain.exec(&txs).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(update1.is_interesting(), "first run should be interesting");

        let exec2 = chain.exec(&txs).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        assert!(
            !update2.is_interesting(),
            "identical second run should not be interesting"
        );
    }

    /// A function that returns early (skip) must not be marked as interesting
    /// when called a second time with different args that hit the same skip path.
    #[test]
    fn coverage_skip_not_interesting_twice() {
        let contract = load_coverage_fixture("src/CoverageSkip.sol:CoverageSkip");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        // First call: skip path (x > 100). Should be interesting (new edges).
        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageSkip::skipOrWorkCall::new((U256::from(200),)).abi_encode(),
        ))];
        let exec1 = chain.exec(&txs1).unwrap();
        assert!(exec1.results[0].success, "first call should succeed");
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(
            update1.is_interesting(),
            "first skip call should be interesting (new edges)"
        );

        // Second call: same skip path with different args (x = 300).
        // Should NOT be interesting under binary edge coverage.
        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageSkip::skipOrWorkCall::new((U256::from(300),)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        assert!(exec2.results[0].success, "second call should succeed");
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        assert!(
            !update2.is_interesting(),
            "second skip call with same code path should not be interesting"
        );
    }

    /// Deploying the same child contract twice via CREATE creates distinct
    /// coverage entries per address.
    #[test]
    fn coverage_same_contract_deployed_twice() {
        let contract = load_coverage_fixture("src/CoverageDeploy.sol:CoverageDeploy");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDeploy::deployChildCall::new(()).abi_encode(),
        ))];
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(update1.is_interesting(), "first run should be interesting");
        let count_after_1 = global.contract_count();
        println!("contract count after 1: {}", count_after_1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageDeploy::deployChildCall::new(()).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        println!("update2: {:?}", update2);
        let count_after_2 = global.contract_count();
        println!("contract count after 2: {}", count_after_2);

        assert_eq!(
            count_after_1, 2,
            "first deployment should have factory + one child"
        );
        assert_eq!(
            count_after_2, 3,
            "deploying same contract at new address should increase count"
        );
        assert!(
            update2.is_interesting(),
            "second deployment at new address should be interesting"
        );
    }

    /// Regression test: initcode coverage recorded during a successful CREATE
    /// must not be kept, while runtime coverage is per-address.
    #[test]
    fn coverage_initcode_removed_on_successful_create() {
        let contract =
            load_coverage_fixture("src/CoverageInitcodeFactory.sol:CoverageInitcodeFactory");
        let (mut chain, target) = deploy_and_setup(&contract);

        let global = SharedCoverage::new();

        let txs1 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageInitcodeFactory::createChildCall::new((U256::from(1),)).abi_encode(),
        ))];
        let exec1 = chain.exec(&txs1).unwrap();
        let coverage1 = exec1.coverage.expect("coverage must be present");
        let update1 = global.merge(&coverage1);
        assert!(update1.is_interesting(), "first run should be interesting");
        let count_after_1 = global.contract_count();
        println!("contract count after 1: {}", count_after_1);

        let txs2 = vec![Transaction::new(target).calldata(Bytes::from(
            CoverageInitcodeFactory::createChildCall::new((U256::from(2),)).abi_encode(),
        ))];
        let exec2 = chain.exec(&txs2).unwrap();
        let coverage2 = exec2.coverage.expect("coverage must be present");
        let update2 = global.merge(&coverage2);
        println!("update2: {:?}", update2);
        let count_after_2 = global.contract_count();
        println!("contract count after 2: {}", count_after_2);

        // With per-address runtime, the second deployment creates a new child at
        // a new address, so it is interesting. Initcode remains discarded.
        assert_eq!(
            count_after_1, 2,
            "first deployment should have factory + one child"
        );
        assert_eq!(
            count_after_2, 3,
            "second deployment at new address should increase count per-address"
        );
        assert!(
            update2.is_interesting(),
            "second deployment at new address should be interesting via runtime"
        );
    }
}
