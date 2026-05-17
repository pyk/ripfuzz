//! Inspector that intercepts Foundry-compatible cheatcodes.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::ContextTr,
    database::InMemoryDB,
    handler::FrameResult,
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput, Interpreter,
        interpreter::EthInterpreter,
    },
    primitives::Address,
};

use crate::chain::cheatcodes::{
    CheatcodeState, VM_ADDRESS, build_outcome, dispatch_effects, effect::apply_effect,
    revert_outcome,
};

/// Inspector that intercepts Foundry-compatible cheatcodes.
#[derive(Debug)]
pub struct CheatcodeInspector {
    pub state: CheatcodeState,
    pub shared_labels: Option<Arc<RwLock<HashMap<Address, String>>>>,
    /// Current EVM call depth (increments in `frame_start`, decrements in
    /// `call_end`/`create_end`).
    pub depth: u64,
}

impl CheatcodeInspector {
    pub fn new() -> Self {
        Self {
            state: CheatcodeState::default(),
            shared_labels: None,
            depth: 0,
        }
    }

    pub fn from_state(state: CheatcodeState) -> Self {
        Self {
            state,
            shared_labels: None,
            depth: 0,
        }
    }

    pub fn with_shared_labels(mut self, labels: Arc<RwLock<HashMap<Address, String>>>) -> Self {
        self.shared_labels = Some(labels);
        self
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
        let curr_depth = self.depth;
        let original_caller = inputs.caller;

        // Collect all info first to avoid borrow issues.
        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| curr_depth > s.set_depth)
            .map(|s| (s.caller, s.origin, !s.used));

        let prank_info = self
            .state
            .prank
            .active
            .as_ref()
            .filter(|p| {
                original_caller == p.prank_caller
                    && curr_depth > p.set_depth
                    && inputs.target_address != VM_ADDRESS
            })
            .map(|p| (p.caller, p.origin, p.single_call, !p.used));

        // Apply startPrank.
        if let Some((caller, _, _)) = start_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o), _)) = start_info {
            self.patch_origin(ctx, o);
        }

        // Apply single-call prank.
        if let Some((caller, _, _, _)) = prank_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o), _, _)) = prank_info {
            self.patch_origin(ctx, o);
        }
        if let Some((_, _, true, _)) = prank_info {
            self.state.prank.active = None;
        }

        // Mark used after all borrows are dropped.
        if start_info.map(|(_, _, needs)| needs).unwrap_or(false)
            && let Some(ref mut start) = self.state.prank.start
        {
            start.used = true;
        }
        if prank_info.map(|(_, _, _, needs)| needs).unwrap_or(false)
            && let Some(ref mut prank) = self.state.prank.active
        {
            prank.used = true;
        }
    }

    /// Apply an active prank to a contract deployment (CREATE) frame.
    fn apply_create_prank(&mut self, inputs: &mut CreateInputs) {
        let curr_depth = self.depth;
        let original_caller = inputs.caller();

        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| curr_depth > s.set_depth)
            .map(|s| (s.caller, !s.used));

        let prank_info = self
            .state
            .prank
            .active
            .as_ref()
            .filter(|p| original_caller == p.prank_caller && curr_depth > p.set_depth)
            .map(|p| (p.caller, p.single_call, !p.used));

        if let Some((caller, _)) = start_info {
            inputs.set_call(caller);
        }

        if let Some((caller, _, _)) = prank_info {
            inputs.set_call(caller);
        }
        if let Some((_, true, _)) = prank_info {
            self.state.prank.active = None;
        }

        if start_info.map(|(_, needs)| needs).unwrap_or(false)
            && let Some(ref mut start) = self.state.prank.start
        {
            start.used = true;
        }
        if prank_info.map(|(_, _, needs)| needs).unwrap_or(false)
            && let Some(ref mut prank) = self.state.prank.active
        {
            prank.used = true;
        }
    }
}

impl Default for CheatcodeInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl<
    CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv, Tx = TxEnv>
        + ContextSetters
        + crate::chain::cheatcodes::effect::CfgMut,
> Inspector<CTX, EthInterpreter> for CheatcodeInspector
{
    fn initialize_interp(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
    }

    fn step(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn step_end(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn frame_start(&mut self, ctx: &mut CTX, frame_input: &mut FrameInput) -> Option<FrameResult> {
        self.depth += 1;
        match frame_input {
            FrameInput::Call(inputs) => self.apply_prank(ctx, inputs),
            FrameInput::Create(inputs) => self.apply_create_prank(inputs),
            FrameInput::Empty => {}
        }
        None
    }

    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(ctx.local());
        if inputs.target_address != VM_ADDRESS || input.len() < 4 {
            return None;
        }

        let sel: [u8; 4] = crate::result_to_option(input[..4].try_into())?;
        let effects = dispatch_effects(sel, &input)?;

        for effect in &effects {
            if let Err(reason) = apply_effect(effect, ctx, &mut self.state) {
                return Some(revert_outcome(&reason));
            }
        }

        // Patch prank_caller and set_depth for pranks configured in this
        // cheatcode call so frame_start knows which parent frame they belong
        // to and which contract initiated them.
        // Note: frame_start is called BEFORE call() for inner frames, so
        // self.depth here is the depth of the inner frame (e.g. the VM
        // precompile call).  The prank initiator is the parent frame, hence
        // parent_depth.
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

        // Sync any newly added labels to the shared map used by the trace
        // inspector.
        if let Some(ref shared) = self.shared_labels
            && let Ok(mut guard) = shared.write()
        {
            guard.clone_from(&self.state.labels);
        }

        let mut outcome = build_outcome(&effects, inputs.gas_limit, ctx, &self.state);
        outcome.memory_offset = inputs.return_memory_offset.clone();
        Some(outcome)
    }

    fn call_end(&mut self, ctx: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        // Discard an unused single-call prank when the frame that created it ends.
        if let Some(ref p) = self.state.prank.active
            && p.single_call
            && self.depth == p.set_depth
        {
            self.state.prank.active = None;
        }
        self.depth = self.depth.saturating_sub(1);
        // Restore tx.origin only when the prank initiator frame exits and no
        // other prank is active.
        if self.depth == 0 || self.state.prank.active.is_none() {
            self.maybe_restore_origin(ctx);
        }
    }

    fn create(&mut self, _context: &mut CTX, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        None
    }

    fn create_end(&mut self, ctx: &mut CTX, _inputs: &CreateInputs, _outcome: &mut CreateOutcome) {
        self.depth = self.depth.saturating_sub(1);
        self.maybe_restore_origin(ctx);
    }
}
