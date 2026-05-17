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
    /// `(original tx.origin, depth_at_which_patched)` — used to restore
    /// `tx.origin` when a single-call prank frame exits.
    pub original_origin: Option<(Address, u64)>,
}

impl CheatcodeInspector {
    pub fn new() -> Self {
        Self {
            state: CheatcodeState::default(),
            shared_labels: None,
            depth: 0,
            original_origin: None,
        }
    }

    pub fn from_state(state: CheatcodeState) -> Self {
        Self {
            state,
            shared_labels: None,
            depth: 0,
            original_origin: None,
        }
    }

    pub fn with_shared_labels(mut self, labels: Arc<RwLock<HashMap<Address, String>>>) -> Self {
        self.shared_labels = Some(labels);
        self
    }

    /// Patch `tx.origin` and remember the original so it can be restored.
    fn patch_origin(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>, new_origin: Address) {
        if self.original_origin.is_none() {
            self.original_origin = Some((ctx.tx().caller, self.depth));
        }
        let mut tx = ctx.tx().clone();
        tx.caller = new_origin;
        ctx.set_tx(tx);
    }

    /// Restore `tx.origin` if we have returned to a depth shallower than
    /// where we patched it and `startPrank` is no longer active.
    fn restore_origin(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>) {
        if self.state.prank.start.is_some() {
            return;
        }
        if let Some((original, patched_at)) = self.original_origin
            && self.depth <= patched_at
        {
            let mut tx = ctx.tx().clone();
            tx.caller = original;
            ctx.set_tx(tx);
            self.original_origin = None;
        }
    }

    /// Apply an active prank to `inputs.caller` and `tx.origin`.
    fn apply_prank(&mut self, ctx: &mut impl ContextSetters<Tx = TxEnv>, inputs: &mut CallInputs) {
        // startPrank: applies to every call deeper than the frame that
        // configured it (Medusa semantics).
        if let Some(ref start_prank) = self.state.prank.start
            && self.depth > start_prank.set_depth + 1
        {
            let origin = start_prank.origin;
            let caller = start_prank.caller;
            self.patch_origin(ctx, origin);
            inputs.caller = caller;
            return;
        }
        // Single-call prank / prankHere.
        if let Some(ref prank) = self.state.prank.active
            && (if prank.here {
                // prankHere: only the very next direct child call from the
                // frame where it was set.
                self.depth == prank.set_depth + 1
            } else {
                true
            })
        {
            let origin = prank.origin;
            let caller = prank.caller;
            let single_call = prank.single_call;
            self.patch_origin(ctx, origin);
            inputs.caller = caller;
            if single_call {
                self.state.prank.active = None;
            }
        }
    }
}

impl Default for CheatcodeInspector {
    fn default() -> Self {
        Self::new()
    }
}

impl<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv, Tx = TxEnv> + ContextSetters>
    Inspector<CTX, EthInterpreter> for CheatcodeInspector
{
    fn initialize_interp(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
    }

    fn step(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn step_end(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn frame_start(&mut self, ctx: &mut CTX, frame_input: &mut FrameInput) -> Option<FrameResult> {
        self.depth += 1;
        if let FrameInput::Call(inputs) = frame_input {
            self.apply_prank(ctx, inputs);
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

        // Patch set_depth for pranks configured in this call so frame_start
        // knows which parent frame they belong to.
        let parent_depth = self.depth.saturating_sub(1);
        if let Some(ref mut prank) = self.state.prank.active
            && prank.here
            && prank.set_depth == 0
        {
            prank.set_depth = parent_depth;
        }
        if let Some(ref mut start) = self.state.prank.start
            && start.set_depth == 0
        {
            start.set_depth = parent_depth;
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
        self.restore_origin(ctx);
        // If we are returning to the frame that configured startPrank,
        // clear it so it does not leak into subsequent top-level sequences.
        if let Some(ref start_prank) = self.state.prank.start
            && self.depth.saturating_sub(1) <= start_prank.set_depth
        {
            self.state.prank.start = None;
        }
        self.depth = self.depth.saturating_sub(1);
    }

    fn create(&mut self, _context: &mut CTX, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        None
    }

    fn create_end(&mut self, ctx: &mut CTX, _inputs: &CreateInputs, _outcome: &mut CreateOutcome) {
        self.restore_origin(ctx);
        self.depth = self.depth.saturating_sub(1);
    }
}
