//! Inspector that intercepts Foundry-compatible cheatcodes.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::ContextTr,
    handler::FrameResult,
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, FrameInput, Interpreter,
        interpreter::EthInterpreter,
    },
    primitives::Address,
};

use crate::vm::{
    ExecutionState, VM_ADDRESS, build_outcome, dispatch_effects,
    effect::{CheatcodeEffect, apply_effect},
    revert_outcome,
};

/// Inspector that intercepts Foundry-compatible cheatcodes.
#[derive(Debug)]
pub struct CheatcodeInspector {
    pub state: ExecutionState,
    pub shared_labels: Option<Arc<RwLock<HashMap<Address, String>>>>,
    /// Current EVM call depth (increments in `frame_start`, decrements in
    /// `call_end`/`create_end`).
    pub depth: u64,
}

impl CheatcodeInspector {
    pub fn new() -> Self {
        Self {
            state: ExecutionState::default(),
            shared_labels: None,
            depth: 0,
        }
    }

    pub fn from_state(state: ExecutionState) -> Self {
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
        if self.state.prank.start.is_none() && self.state.prank.active.is_none() {
            return;
        }
        let curr_depth = self.depth;
        let original_caller = inputs.caller;

        // Collect all info first to avoid borrow issues.
        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| curr_depth > s.set_depth && inputs.target_address != VM_ADDRESS)
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

        // Apply startPrank.
        if let Some((caller, _)) = start_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o))) = start_info {
            self.patch_origin(ctx, o);
        }

        // Apply single-call prank.
        if let Some((caller, _)) = prank_info {
            inputs.caller = caller;
        }
        if let Some((_, Some(o))) = prank_info {
            self.patch_origin(ctx, o);
        }

        // Mark used after all borrows are dropped.
        // The prank is *not* cleared here; it stays in `active` so we know
        // when to restore tx.origin (when the initiator frame exits).
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

        // Collect all info first to avoid borrow issues.
        let start_info = self
            .state
            .prank
            .start
            .as_ref()
            .filter(|s| curr_depth > s.set_depth)
            .map(|s| (s.caller, s.origin));

        let prank_info = self
            .state
            .prank
            .active
            .as_ref()
            .filter(|p| original_caller == p.prank_caller && curr_depth > p.set_depth && !p.used)
            .map(|p| (p.caller, p.origin));

        // Apply startPrank.
        if let Some((caller, _)) = start_info {
            inputs.set_call(caller);
        }
        if let Some((_, Some(o))) = start_info {
            self.patch_origin(ctx, o);
        }

        // Apply single-call prank.
        if let Some((caller, _)) = prank_info {
            inputs.set_call(caller);
        }
        if let Some((_, Some(o))) = prank_info {
            self.patch_origin(ctx, o);
        }

        // Mark used after all borrows are dropped.
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

impl Default for CheatcodeInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX: ContextTr<Block = BlockEnv, Tx = TxEnv> + ContextSetters + crate::vm::effect::CfgMut>
    Inspector<CTX, EthInterpreter> for CheatcodeInspector
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

        // If stopPrank was called, restore tx.origin immediately.
        if effects
            .iter()
            .any(|e| matches!(e, CheatcodeEffect::ClearPrank))
        {
            self.maybe_restore_origin(ctx);
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
        // When the frame that initiated a single-call prank exits, clear the
        // prank and restore origin if no other prank is active.
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
