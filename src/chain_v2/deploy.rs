//! Target contract deployment.

use revm::{
    Database as _, MainBuilder, MainContext,
    context::{Context, TxEnv},
    handler::ExecuteCommitEvm,
    primitives::{Address, TxKind, U256},
};

use crate::chain_v2::error::{DeployError, decode_solidity_error};
use crate::chain_v2::state::StateSnapshot;
use crate::target::Contract;

/// Deploy a contract and return the deployed address + updated state.
pub fn execute(
    mut state: StateSnapshot,
    target: &Contract,
    deploy_value: U256,
) -> Result<(Address, StateSnapshot), DeployError> {
    let mut ctx = Context::mainnet().with_db(state.db.clone());
    ctx.block = state.block_env.clone();
    ctx.cfg = state.cfg_env.clone();
    ctx.cfg.tx_gas_limit_cap = Some(u64::MAX);

    let mut evm = ctx.build_mainnet();

    let tx = TxEnv {
        caller: state.deployer,
        kind: TxKind::Create,
        data: target.initcode.clone(),
        gas_limit: u64::MAX,
        value: deploy_value,
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
                return Err(DeployError::Reverted { reason, output });
            }
            revm::context::result::ExecutionResult::Halt { reason, .. } => {
                return Err(DeployError::Halt {
                    reason: format!("{reason}"),
                });
            }
            _ => {
                return Err(DeployError::Other(anyhow::anyhow!("deployment failed")));
            }
        }
    }

    let address = result.created_address().ok_or(DeployError::NoAddress)?;

    let db = evm.ctx.journaled_state.database;
    state.db = db;

    // Read back the deployer nonce from committed state.
    let nonce = state
        .db
        .basic(state.deployer)
        .unwrap_or_default()
        .unwrap_or_default()
        .nonce;
    state.deployer_nonce = nonce;

    Ok((address, state))
}
