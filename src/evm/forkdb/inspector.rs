//! Inspector that marks locally-created addresses so ForkDB skips RPC.
//!
//! During CREATE / CREATE2 EVM operations, the [`LocalTracker`] inspector
//! computes the `created_address` from the caller's current nonce and
//! registers it with the shared [`SharedLocalAddressRegistry`]. This
//! happens *before* `journal.load_account(created_address)` inside the
//! CREATE handler, so `ForkDB::basic_ref` sees the address as local and
//! returns `None` without an RPC fetch.

use revm::{
    context_interface::ContextTr,
    handler::FrameResult,
    inspector::JournalExt,
    interpreter::{
        CreateInputs, CreateOutcome, FrameInput, Interpreter, interpreter::EthInterpreter,
    },
};

use crate::evm::forkdb::SharedLocalAddressRegistry;

/// EVM inspector that tracks locally-created addresses for fork mode.
///
/// Every CREATE/CREATE2 opcode's target address is computed from the
/// caller's nonce (read from the journaled state) and marked as local
/// via [`SharedLocalAddressRegistry::mark_local`].
///
/// Cloning is cheap (shares the same inner registry).
#[derive(Debug, Clone)]
pub struct LocalTracker {
    registry: SharedLocalAddressRegistry,
}

impl LocalTracker {
    /// Create a new `LocalTracker` that writes to the given registry.
    pub fn new(registry: SharedLocalAddressRegistry) -> Self {
        Self { registry }
    }

    /// Compute the address that `inputs.caller()` would create with its
    /// current nonce (as seen in the journal state), and mark it local.
    fn mark_from_inputs(&self, ctx: &impl JournalExt, inputs: &CreateInputs) {
        let state = ctx.evm_state();
        let nonce = state
            .get(&inputs.caller())
            .map(|account| account.info.nonce)
            .unwrap_or(0);
        let created = inputs.created_address(nonce);
        self.registry.mark_local(created);
    }
}

impl<CTX> revm::Inspector<CTX, EthInterpreter, FrameInput, FrameResult> for LocalTracker
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn create(&mut self, ctx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.mark_from_inputs(ctx.journal(), inputs);
        None
    }

    // No-op hooks (required by the trait but unnecessary for our purpose).
    fn initialize_interp(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
    }

    fn step(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}

    fn step_end(&mut self, _interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {}
}
