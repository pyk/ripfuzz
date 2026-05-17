//! Cheatcode extension point for Foundry-compatible precompiles.
//!
//! Scoped to Medusa's standard cheatcode set. Each cheatcode category lives in
//! its own file and registers itself into a dispatch table.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use alloy_dyn_abi::DynSolValue;
use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::ContextTr,
    database::InMemoryDB,
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Gas, InstructionResult, Interpreter,
        InterpreterResult, interpreter::EthInterpreter,
    },
    primitives::{Address, Bytes, U256},
};

pub mod account;
pub mod assert;
pub mod ffi;
pub mod label;
pub mod prank;
pub mod snapshot;
pub mod state;
pub mod string;
pub mod wallet;

/// Foundry cheatcode VM contract address.
pub const VM_ADDRESS: Address = Address::new([
    0x71, 0x09, 0x70, 0x9e, 0xcf, 0xa9, 0x1a, 0x80, 0x62, 0x6f, 0xf3, 0x98, 0x9d, 0x68, 0xf6, 0x7f,
    0x5b, 0x1d, 0xd1, 0x2d,
]);

/// State accumulated by cheatcodes during execution.
#[derive(Clone, Debug, Default)]
pub struct CheatcodeState {
    pub labels: HashMap<Address, String>,
    pub prank: Option<PrankState>,
    pub start_prank: Option<StartPrankState>,
    pub snapshots: Vec<revm::database::InMemoryDB>,
    pub ffi_enabled: bool,
    pub warp_timestamp: Option<U256>,
    pub roll_number: Option<U256>,
    pub fee: Option<U256>,
    pub coinbase: Option<Address>,
    pub prevrandao: Option<[u8; 32]>,
    pub chain_id: Option<U256>,
}

#[derive(Debug, Clone)]
pub struct PrankState {
    pub caller: Address,
    pub origin: Address,
    pub single_call: bool,
    /// Call depth of the frame that configured this prank.
    pub set_depth: u64,
    /// `true` for `prankHere` semantics: applies only to the very next
    /// direct child call from the frame where it was set.
    pub here: bool,
}

#[derive(Debug, Clone)]
pub struct StartPrankState {
    pub caller: Address,
    pub origin: Address,
}

/// Inspector that intercepts Foundry-compatible cheatcodes.
#[derive(Debug)]
pub struct CheatcodeInspector {
    pub state: CheatcodeState,
    pub shared_labels: Option<Arc<RwLock<HashMap<Address, String>>>>,
    /// Current EVM call depth (increments in `call`/`create`, decrements in
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
        if self.state.start_prank.is_some() {
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
        if let Some(ref start_prank) = self.state.start_prank {
            let origin = start_prank.origin;
            let caller = start_prank.caller;
            self.patch_origin(ctx, origin);
            inputs.caller = caller;
            return;
        }
        if let Some(ref prank) = self.state.prank
            && (if prank.here {
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
                self.state.prank = None;
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

    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.depth += 1;
        self.apply_prank(ctx, inputs);

        let input = inputs.input.bytes_local(ctx.local());
        if inputs.target_address != VM_ADDRESS || input.len() < 4 {
            return None;
        }

        let sel: [u8; 4] = crate::result_to_option(input[..4].try_into())?;

        let mut outcome = match sel {
            // Block / state manipulation
            state::WARP_SELECTOR => state::handle_warp(self, ctx, &input),
            state::ROLL_SELECTOR => state::handle_roll(self, ctx, &input),
            state::FEE_SELECTOR => state::handle_fee(self, ctx, &input),
            state::COINBASE_SELECTOR => state::handle_coinbase(self, ctx, &input),
            state::DIFFICULTY_SELECTOR => Some(dummy_success()),
            state::PREVRANDAO_SELECTOR => state::handle_prevrandao(self, ctx, &input),
            state::CHAIN_ID_SELECTOR => state::handle_chain_id(self, ctx, &input),

            // Account manipulation
            account::DEAL_SELECTOR => account::handle_deal(self, ctx, &input),
            account::ETCH_SELECTOR => account::handle_etch(self, ctx, &input),
            account::SET_NONCE_SELECTOR => account::handle_set_nonce(self, ctx, &input),
            account::GET_NONCE_SELECTOR => account::handle_get_nonce(self, ctx, &input),
            account::LOAD_SELECTOR => account::handle_load(self, ctx, &input),
            account::STORE_SELECTOR => account::handle_store(self, ctx, &input),

            // Prank
            prank::PRANK_SELECTOR => prank::handle_prank(self, &input),
            prank::PRANK_HERE_SELECTOR => prank::handle_prank_here(self, &input),
            prank::START_PRANK_SELECTOR => prank::handle_start_prank(self, &input),
            prank::STOP_PRANK_SELECTOR => prank::handle_stop_prank(self),

            // Snapshot
            snapshot::SNAPSHOT_SELECTOR => snapshot::handle_snapshot(self, ctx),
            snapshot::REVERT_TO_SELECTOR => snapshot::handle_revert_to(self, ctx, &input),

            // Label
            label::LABEL_SELECTOR => label::handle_label(self, ctx, &input),

            // Assertions
            assert::ASSERT_TRUE_SELECTOR => assert::handle_assert_true(self, &input),
            assert::ASSERT_FALSE_SELECTOR => assert::handle_assert_false(self, &input),
            assert::ASSERT_EQ_BOOL_SELECTOR => assert::handle_assert_eq_bool(self, &input),
            assert::ASSERT_EQ_UINT_SELECTOR => assert::handle_assert_eq_uint(self, &input),
            assert::ASSERT_EQ_INT_SELECTOR => assert::handle_assert_eq_int(self, &input),
            assert::ASSERT_EQ_ADDRESS_SELECTOR => assert::handle_assert_eq_address(self, &input),
            assert::ASSERT_EQ_BYTES32_SELECTOR => assert::handle_assert_eq_bytes32(self, &input),
            assert::ASSERT_EQ_STRING_SELECTOR => assert::handle_assert_eq_string(self, &input),
            assert::ASSERT_EQ_BYTES_SELECTOR => assert::handle_assert_eq_bytes(self, &input),
            assert::ASSERT_NOT_EQ_BOOL_SELECTOR => assert::handle_assert_not_eq_bool(self, &input),
            assert::ASSERT_NOT_EQ_UINT_SELECTOR => assert::handle_assert_not_eq_uint(self, &input),
            assert::ASSERT_NOT_EQ_INT_SELECTOR => assert::handle_assert_not_eq_int(self, &input),
            assert::ASSERT_NOT_EQ_ADDRESS_SELECTOR => {
                assert::handle_assert_not_eq_address(self, &input)
            }
            assert::ASSERT_NOT_EQ_BYTES32_SELECTOR => {
                assert::handle_assert_not_eq_bytes32(self, &input)
            }
            assert::ASSERT_NOT_EQ_STRING_SELECTOR => {
                assert::handle_assert_not_eq_string(self, &input)
            }
            assert::ASSERT_NOT_EQ_BYTES_SELECTOR => {
                assert::handle_assert_not_eq_bytes(self, &input)
            }
            assert::ASSERT_LT_UINT_SELECTOR => assert::handle_assert_lt_uint(self, &input),
            assert::ASSERT_LT_INT_SELECTOR => assert::handle_assert_lt_int(self, &input),
            assert::ASSERT_LE_UINT_SELECTOR => assert::handle_assert_le_uint(self, &input),
            assert::ASSERT_LE_INT_SELECTOR => assert::handle_assert_le_int(self, &input),
            assert::ASSERT_GT_UINT_SELECTOR => assert::handle_assert_gt_uint(self, &input),
            assert::ASSERT_GT_INT_SELECTOR => assert::handle_assert_gt_int(self, &input),
            assert::ASSERT_GE_UINT_SELECTOR => assert::handle_assert_ge_uint(self, &input),
            assert::ASSERT_GE_INT_SELECTOR => assert::handle_assert_ge_int(self, &input),

            // String / type conversion
            string::TO_STRING_ADDRESS_SELECTOR => string::handle_to_string_address(self, &input),
            string::TO_STRING_BOOL_SELECTOR => string::handle_to_string_bool(self, &input),
            string::TO_STRING_UINT_SELECTOR => string::handle_to_string_uint(self, &input),
            string::TO_STRING_INT_SELECTOR => string::handle_to_string_int(self, &input),
            string::TO_STRING_BYTES32_SELECTOR => string::handle_to_string_bytes32(self, &input),
            string::TO_STRING_BYTES_SELECTOR => string::handle_to_string_bytes(self, &input),
            string::PARSE_UINT_SELECTOR => string::handle_parse_uint(self, &input),
            string::PARSE_INT_SELECTOR => string::handle_parse_int(self, &input),
            string::PARSE_BOOL_SELECTOR => string::handle_parse_bool(self, &input),
            string::PARSE_ADDRESS_SELECTOR => string::handle_parse_address(self, &input),
            string::PARSE_BYTES_SELECTOR => string::handle_parse_bytes(self, &input),
            string::PARSE_BYTES32_SELECTOR => string::handle_parse_bytes32(self, &input),
            string::GET_CODE_SELECTOR => string::handle_get_code(self, &input),

            // Wallet / crypto
            wallet::ADDR_SELECTOR => wallet::handle_addr(self, &input),
            wallet::SIGN_SELECTOR => wallet::handle_sign(self, &input),

            // FFI
            ffi::FFI_SELECTOR => ffi::handle_ffi(self, &input),

            // Unknown VM call: silently drop.
            _ => Some(dummy_success()),
        }?;
        // Cheatcode calls short-circuit the EVM.  Preserve the caller's gas
        // so the parent frame does not lose gas on every cheatcode invocation.
        outcome.result.gas = Gas::new(inputs.gas_limit);
        Some(outcome)
    }

    fn call_end(&mut self, ctx: &mut CTX, _inputs: &CallInputs, _outcome: &mut CallOutcome) {
        self.restore_origin(ctx);
        self.depth = self.depth.saturating_sub(1);
    }

    fn create(&mut self, _context: &mut CTX, _inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.depth += 1;
        None
    }

    fn create_end(&mut self, ctx: &mut CTX, _inputs: &CreateInputs, _outcome: &mut CreateOutcome) {
        self.restore_origin(ctx);
        self.depth = self.depth.saturating_sub(1);
    }
}

/// Build a silent success outcome for a short-circuited cheatcode call.
pub(crate) fn dummy_success() -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Stop,
            output: Bytes::new(),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

/// Build a revert outcome with a string reason.
pub(crate) fn revert_outcome(reason: &str) -> CallOutcome {
    let mut encoded = vec![0x08, 0xc3, 0x79, 0xa0];
    encoded.extend_from_slice(&DynSolValue::String(reason.into()).abi_encode());
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Revert,
            output: Bytes::from(encoded),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

/// Build a success outcome returning a single uint256 value.
pub(crate) fn success_u256_outcome(value: U256) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(value.to_be_bytes_vec()),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

/// Build a success outcome returning a single bool value.
pub(crate) fn success_bool_outcome(value: bool) -> CallOutcome {
    let mut output = vec![0u8; 32];
    if value {
        output[31] = 1;
    }
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(output),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

/// Build a success outcome returning raw bytes.
pub(crate) fn success_bytes_outcome(bytes: Vec<u8>) -> CallOutcome {
    CallOutcome {
        result: InterpreterResult {
            result: InstructionResult::Return,
            output: Bytes::from(bytes),
            gas: Gas::new(0),
        },
        memory_offset: 0..0,
        was_precompile_called: false,
        precompile_call_logs: Vec::new(),
    }
}

/// Decode a single `uint256` argument from calldata after the selector.
pub(crate) fn decode_u256_arg(input: &Bytes) -> Option<U256> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(U256::from_be_slice(&input[4..36]))
}

/// Decode a single `address` argument from calldata after the selector.
pub(crate) fn decode_address_arg(input: &Bytes) -> Option<Address> {
    if input.len() < 4 + 32 {
        return None;
    }
    Some(Address::from_slice(&input[4 + 12..4 + 32]))
}

/// Decode an `(address, uint256)` pair from calldata after the selector.
pub(crate) fn decode_address_u256_args(input: &Bytes) -> Option<(Address, U256)> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let value = U256::from_be_slice(&input[4 + 32..4 + 64]);
    Some((addr, value))
}

/// Decode an `(address, bytes32, bytes32)` triple from calldata.
pub(crate) fn decode_address_bytes32_bytes32_args(
    input: &Bytes,
) -> Option<(Address, [u8; 32], [u8; 32])> {
    if input.len() < 4 + 96 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    let mut value = [0u8; 32];
    value.copy_from_slice(&input[4 + 64..4 + 96]);
    Some((addr, slot, value))
}

/// Decode an `(address, bytes32)` pair from calldata.
pub(crate) fn decode_address_bytes32_args(input: &Bytes) -> Option<(Address, [u8; 32])> {
    if input.len() < 4 + 64 {
        return None;
    }
    let addr = Address::from_slice(&input[4 + 12..4 + 32]);
    let mut slot = [0u8; 32];
    slot.copy_from_slice(&input[4 + 32..4 + 64]);
    Some((addr, slot))
}
