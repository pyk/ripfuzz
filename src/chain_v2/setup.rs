//! Optional `setup()` execution after deployment.

use revm::{
    Database as _, MainBuilder, MainContext,
    context::{Context, TxEnv},
    handler::ExecuteCommitEvm,
    primitives::{Address, Bytes, TxKind},
};

use crate::chain_v2::error::{SetupError, decode_solidity_error};
use crate::chain_v2::state::StateSnapshot;
use crate::target::Contract;

const SETUP_SELECTOR: [u8; 4] = [0xba, 0x0b, 0xba, 0x40];

/// Run the target contract's optional `setup()` and return updated state.
pub fn execute(
    mut state: StateSnapshot,
    contract_address: Option<Address>,
    target: &Contract,
) -> Result<StateSnapshot, SetupError> {
    let contract_address = contract_address
        .ok_or_else(|| SetupError::Other(anyhow::anyhow!("contract not deployed")))?;

    if target.setup_function.is_none() {
        return Ok(state);
    }

    let mut ctx = Context::mainnet().with_db(state.db.clone());
    ctx.block = state.block_env.clone();
    ctx.cfg = state.cfg_env.clone();
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);

    let mut evm = ctx.build_mainnet();

    let tx = TxEnv {
        caller: state.deployer,
        kind: TxKind::Call(contract_address),
        data: Bytes::copy_from_slice(&SETUP_SELECTOR),
        gas_limit: u64::MAX,
        nonce: state.deployer_nonce,
        ..Default::default()
    };

    let result = evm
        .transact_commit(tx)
        .map_err(|e| -> anyhow::Error { e.into() })?;

    if !result.is_success() {
        match result {
            revm::context::result::ExecutionResult::Revert { output, .. } => {
                let reason = decode_solidity_error(&output)
                    .unwrap_or_else(|| format!("reverted (output: 0x{})", hex::encode(&output)));
                return Err(SetupError::Reverted { reason, output });
            }
            revm::context::result::ExecutionResult::Halt { reason, .. } => {
                return Err(SetupError::Halt {
                    reason: format!("{reason}"),
                });
            }
            _ => {
                return Err(SetupError::Other(anyhow::anyhow!("setup failed")));
            }
        }
    }

    let db = evm.ctx.journaled_state.database;
    state.db = db;

    let nonce = state
        .db
        .basic(state.deployer)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    state.deployer_nonce = nonce;

    Ok(state)
}
