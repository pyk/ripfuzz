//! Coverage inspector that records EVM program counter hits into a per-contract local coverage map.

use alloy_primitives::ruint::UintTryTo;
use revm::{
    bytecode::opcode::{JUMP, JUMPI},
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
};

use alloy_primitives::B256;

use crate::evm::coverage::edge::edge_marker;
use crate::evm::coverage::map::{LocalContractCoverage, LocalCoverage};

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
            coverage.reverts[word] |= 1u64 << bit;
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
            coverage.edges[pc] = coverage.edges[pc].saturating_add(1);
        }
        if pc < coverage.depths.len() {
            let depth = self.current_call_depth.min(63);
            coverage.depths[pc] |= 1u64 << depth;
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
    use revm::{context::TxEnv, primitives::TxKind};

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
    use crate::evm::coverage;
    use crate::foundry;
    use crate::target::Contract;

    const GAS_LIMIT: u64 = 16_777_216;

    fn load_fixture(id: &str) -> Contract {
        let project = foundry::Project::new("fixtures/basic-target");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    #[test]
    fn coverage_inspector_collects_hits_for_deployed_contract() {
        let contract = load_fixture("src/NamedMismatch.sol:DifferentName");

        let mut chain = Chain::empty();
        let inspector = coverage::Inspector::new();
        let tx = TxEnv {
            caller: DEFAULT_DEPLOYER,
            kind: TxKind::Create,
            data: contract.initcode,
            gas_limit: GAS_LIMIT,
            ..Default::default()
        };
        let (result, inspector) = chain.inspect(tx, inspector).unwrap();
        assert!(result.success, "deployment should succeed");

        let coverage = inspector.into_coverage();
        assert!(
            !coverage.contracts.is_empty(),
            "coverage should contain at least one contract"
        );
    }
}
