//! Inspector that intercepts Foundry-compatible cheatcodes.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_sol_types::SolInterface;
use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::ContextTr,
    handler::FrameResult,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput, Gas, InstructionResult,
        Interpreter, interpreter::EthInterpreter,
    },
    primitives::Address,
    primitives::hardfork::SpecId,
};

use crate::evm::cheatcode::calls;
use crate::evm::cheatcode::calls::Vm::VmCalls;
use crate::evm::cheatcode::calls::fork::AsForkDatabase;
use crate::evm::cheatcode::{CheatcodeConfig, ExecutionState, VM_ADDRESS};
use crate::evm::database::DatabaseExt;
use crate::evm::forkdb::SharedLocalAddressRegistry;

/// Minimal trait to mutate config fields on generic EVM contexts.
pub trait CfgMut {
    fn set_chain_id(&mut self, chain_id: u64);
    fn set_spec_and_mainnet_gas_params(&mut self, spec: SpecId);
}

impl<BLOCK, TX, DB, JOURNAL, CHAIN, LOCAL> CfgMut
    for revm::context::Context<BLOCK, TX, revm::context::CfgEnv<SpecId>, DB, JOURNAL, CHAIN, LOCAL>
where
    DB: revm::Database,
    JOURNAL: revm::context_interface::JournalTr<Database = DB>,
    LOCAL: revm::context_interface::LocalContextTr,
{
    fn set_chain_id(&mut self, chain_id: u64) {
        self.cfg.chain_id = chain_id;
    }

    fn set_spec_and_mainnet_gas_params(&mut self, spec: SpecId) {
        self.cfg.set_spec_and_mainnet_gas_params(spec);
    }
}

/// Inspector that intercepts Foundry-compatible cheatcodes.
#[derive(Debug)]
pub struct Inspector {
    pub state: ExecutionState,
    pub shared_labels: Option<Arc<RwLock<HashMap<Address, String>>>>,
    pub depth: u64,
    /// Shared registry for marking locally-created addresses (e.g. from
    /// `vm.addr`) so the ForkDB backend skips RPC for them.
    pub local_registry: Option<SharedLocalAddressRegistry>,
}

impl Inspector {
    /// Create a new inspector with the given cheatcode [`CheatcodeConfig`].
    pub fn new(config: CheatcodeConfig) -> Self {
        Self {
            state: ExecutionState::from_config(&config),
            shared_labels: None,
            depth: 0,
            local_registry: None,
        }
    }

    pub fn from_state(state: ExecutionState) -> Self {
        let local_registry = Some(state.local_registry.clone());
        Self {
            state,
            shared_labels: None,
            depth: 0,
            local_registry,
        }
    }

    /// Set the shared local address registry so that `vm.addr` can mark
    /// derived addresses as local.
    pub fn with_local_registry(mut self, registry: SharedLocalAddressRegistry) -> Self {
        self.state.local_registry = registry.clone();
        self.local_registry = Some(registry);
        self
    }

    fn with_default_config() -> Self {
        Self::new(CheatcodeConfig::default())
    }

    /// Patch `tx.origin` and remember the original so it can be restored.
    fn patch_origin(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>, new_origin: Address) {
        if self.state.prank.original_origin.is_none() {
            self.state.prank.original_origin = Some(ctx.tx().caller);
        }
        let mut tx = ctx.tx().clone();
        tx.caller = new_origin;
        ctx.set_tx(tx);
    }

    /// Restore `tx.origin` if no prank is active.
    fn maybe_restore_origin(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>) {
        if self.state.prank.start.is_some() || self.state.prank.active.is_some() {
            return;
        }
        if let Some(orig) = self.state.prank.original_origin {
            let mut tx = ctx.tx().clone();
            tx.caller = orig;
            ctx.set_tx(tx);
            self.state.prank.original_origin = None;
        }
    }

    /// Apply an active prank to a nested call frame.
    fn apply_prank(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>, inputs: &mut CallInputs) {
        if self.state.prank.start.is_none() && self.state.prank.active.is_none() {
            return;
        }
        let curr_depth = self.depth;
        let original_caller = inputs.caller;

        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| {
                original_caller == s.prank_caller
                    && curr_depth > s.set_depth
                    && inputs.target_address != VM_ADDRESS
            })
            .map(|s| (s.caller, s.origin));

        let prank_info = self
            .state
            .prank
            .active
            .as_ref()
            .filter(|p| {
                original_caller == p.prank_caller
                    && curr_depth > p.set_depth
                    && inputs.target_address != VM_ADDRESS
                    && !p.used
            })
            .map(|p| (p.caller, p.origin));

        if let Some((caller, _)) = start_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o))) = start_info {
            self.patch_origin(ctx, o);
        }

        if let Some((caller, _)) = prank_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o))) = prank_info {
            self.patch_origin(ctx, o);
        }

        if start_info.is_some()
            && let Some(ref mut start) = self.state.prank.start
        {
            start.used = true;
        }
        if prank_info.is_some()
            && let Some(ref mut prank) = self.state.prank.active
        {
            prank.used = true;
        }
    }

    /// Apply an active prank to a contract deployment (CREATE) frame.
    fn apply_create_prank(
        &mut self,
        ctx: &mut impl ContextSetters<Tx = TxEnv>,
        inputs: &mut CreateInputs,
    ) {
        if self.state.prank.start.is_none() && self.state.prank.active.is_none() {
            return;
        }
        let curr_depth = self.depth;
        let original_caller = inputs.caller();

        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| original_caller == s.prank_caller && curr_depth > s.set_depth)
            .map(|s| (s.caller, s.origin));

        let prank_info = self
            .state
            .prank
            .active
            .as_ref()
            .filter(|p| original_caller == p.prank_caller && curr_depth > p.set_depth && !p.used)
            .map(|p| (p.caller, p.origin));

        if let Some((caller, _)) = start_info {
            inputs.set_call(caller);
        }
        if let Some((_, Some(o))) = start_info {
            self.patch_origin(ctx, o);
        }

        if let Some((caller, _)) = prank_info {
            inputs.set_call(caller);
        }
        if let Some((_, Some(o))) = prank_info {
            self.patch_origin(ctx, o);
        }

        if start_info.is_some()
            && let Some(ref mut start) = self.state.prank.start
        {
            start.used = true;
        }
        if prank_info.is_some()
            && let Some(ref mut prank) = self.state.prank.active
        {
            prank.used = true;
        }
    }
}

impl Default for Inspector {
    fn default() -> Self {
        Self::with_default_config()
    }
}

impl<CTX: ContextTr<Block = BlockEnv, Tx = TxEnv> + ContextSetters + CfgMut>
    revm::inspector::Inspector<CTX, EthInterpreter> for Inspector
where
    CTX::Db: DatabaseExt + AsForkDatabase,
    CTX::Journal: calls::fork::CommitRemoteBeforeForkSwitch,
{
    fn initialize_interp(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
    }

    fn step(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn step_end(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn frame_start(&mut self, ctx: &mut CTX, frame_input: &mut FrameInput) -> Option<FrameResult> {
        self.depth += 1;
        match frame_input {
            FrameInput::Call(inputs) => self.apply_prank(ctx, inputs),
            FrameInput::Create(inputs) => self.apply_create_prank(ctx, inputs),
            FrameInput::Empty => {}
        }
        None
    }

    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(ctx.local());
        if inputs.target_address != VM_ADDRESS || input.len() < 4 {
            return None;
        }

        let call = VmCalls::abi_decode(&input).ok()?;
        let is_stop_prank = matches!(&call, VmCalls::stopPrank(_));
        let is_addr = matches!(&call, VmCalls::addr(_));
        let mut outcome = calls::dispatch(call, ctx, &mut self.state);
        // Ensure return data is written to the caller's expected memory offset
        // so Solidity can read it from the returndata buffer.
        if let Some(ref mut o) = outcome {
            o.memory_offset = inputs.return_memory_offset.clone();
            o.result.gas = Gas::new(inputs.gas_limit);
        }

        // When `vm.addr` derives a new address, mark it as local so the
        // ForkDB backend skips RPC for it. The address is returned as a
        // 32-byte left-padded value in the return data.
        if is_addr
            && let Some(ref outcome) = outcome
            && outcome.result.result == InstructionResult::Return
            && outcome.result.output.len() >= 32
            && let Some(ref registry) = self.local_registry
        {
            let addr = Address::from_slice(&outcome.result.output[12..32]);
            registry.mark_local(addr);
        }

        let parent_depth = self.depth.saturating_sub(1);
        match self.state.prank.active.as_mut() {
            Some(p) if p.prank_caller == Address::ZERO => {
                p.prank_caller = inputs.caller;
            }
            _ => {}
        }
        match self.state.prank.active.as_mut() {
            Some(p) if p.set_depth == 0 => {
                p.set_depth = parent_depth;
            }
            _ => {}
        }
        match self.state.prank.start.as_mut() {
            Some(s) if s.prank_caller == Address::ZERO => {
                s.prank_caller = inputs.caller;
            }
            _ => {}
        }
        match self.state.prank.start.as_mut() {
            Some(s) if s.set_depth == 0 => {
                s.set_depth = parent_depth;
            }
            _ => {}
        }

        // If stopPrank was called, restore tx.origin immediately.
        if let Some(ref o) = outcome
            && o.result.result == InstructionResult::Stop
            && is_stop_prank
        {
            self.maybe_restore_origin(ctx);
        }

        // Sync labels to shared map.
        if let Some(ref shared) = self.shared_labels
            && let Ok(mut guard) = shared.write()
        {
            guard.clone_from(&self.state.labels);
        }

        outcome
    }

    fn call_end(&mut self, ctx: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        let initiator_exited = self
            .state
            .prank
            .active
            .as_ref()
            .map(|p| p.single_call && self.depth == p.set_depth)
            .unwrap_or(false);
        if initiator_exited {
            self.state.prank.active = None;
        }
        self.depth = self.depth.saturating_sub(1);
        if initiator_exited {
            self.maybe_restore_origin(ctx);
        }
    }

    fn create(&mut self, _context: &mut CTX, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        None
    }

    fn create_end(&mut self, ctx: &mut CTX, _inputs: &CreateInputs, _outcome: &mut CreateOutcome) {
        let initiator_exited = self
            .state
            .prank
            .active
            .as_ref()
            .map(|p| p.single_call && self.depth == p.set_depth)
            .unwrap_or(false);
        if initiator_exited {
            self.state.prank.active = None;
        }
        self.depth = self.depth.saturating_sub(1);
        if initiator_exited {
            self.maybe_restore_origin(ctx);
        }
    }
}
