use std::collections::HashMap;

use alloy_dyn_abi::DynSolType;
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::JsonAbi;
use anyhow::Result;
use revm::{
    MainBuilder, MainContext,
    context::{Context, TxEnv},
    database::InMemoryDB,
    database_interface::Database,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
    state::AccountInfo,
};

use crate::contract::ContractArtifact;
use crate::inspector::CoverageInspector;
use crate::trace::CallTraceInspector;

pub const CALLER: Address = Address::new([0xde; 20]);
pub const GAS_LIMIT: u64 = 1_000_000;

/// Extract a human-readable error message from a failed deployment result.
fn extract_deployment_error(result: &revm::context::result::ExecutionResult) -> String {
    use revm::context::result::ExecutionResult;

    match result {
        ExecutionResult::Success { .. } => "contract returned no address".to_string(),
        ExecutionResult::Revert { output, .. } => {
            if let Some(reason) = decode_solidity_error(output) {
                format!("reverted with '{reason}'")
            } else {
                format!("reverted (output: 0x{})", hex::encode(output))
            }
        }
        ExecutionResult::Halt { reason, .. } => {
            format!("halted: {reason}")
        }
    }
}

/// Decode a Solidity `Error(string)` revert payload.
fn decode_solidity_error(output: &Bytes) -> Option<String> {
    if output.len() < 4 || output[..4] != ERROR_SELECTOR {
        return None;
    }

    let string_type = DynSolType::String;
    let decoded = string_type.abi_decode_params(&output[4..]).ok()?;

    match decoded {
        DynSolValue::String(s) => Some(s),
        _ => None,
    }
}

/// Solidity `Error(string)` selector: `keccak256("Error(string)")[:4]`
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

#[derive(Debug)]
pub struct EvmRunner {
    pub contract_address: Address,
    pub deployed_db: InMemoryDB,
}

impl EvmRunner {
    pub fn from_target(target: &ContractArtifact) -> Result<Self> {
        let mut db = InMemoryDB::default();

        db.insert_account_info(
            CALLER,
            AccountInfo {
                balance: U256::from(1_000_000_000_000_000_000u128),
                nonce: 0,
                code_hash: KECCAK_EMPTY,
                code: None,
                account_id: None,
            },
        );
        crate::trace::insert_foundry_vm(&mut db);

        let initcode_map: HashMap<Bytes, (String, JsonAbi)> = target
            .all_contracts
            .iter()
            .map(|(name, (initcode, abi))| (initcode.clone(), (name.clone(), abi.clone())))
            .collect();
        let inspector = CallTraceInspector::new(initcode_map);
        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let tx = TxEnv {
            caller: CALLER,
            kind: TxKind::Create,
            data: target.initcode.clone(),
            gas_limit: GAS_LIMIT,
            ..Default::default()
        };

        let result = evm.inspect_tx_commit(tx)?;
        let contract_address = result.created_address().ok_or_else(|| {
            let reason = extract_deployment_error(&result);
            let trace = evm.inspector.format();
            anyhow::anyhow!("deployment failed: {reason}\n\nTrace:\n{trace}")
        })?;

        let deployed_db = evm.ctx.journaled_state.database;
        Ok(Self {
            contract_address,
            deployed_db,
        })
    }

    pub fn run_sequence(&self, input: &[u8]) -> Result<bool, anyhow::Error> {
        let mut db = self.deployed_db.clone();
        let start_nonce = db
            .basic(CALLER)
            .map_err(|_| anyhow::anyhow!("db error"))?
            .unwrap_or_default()
            .nonce;

        let inspector = CoverageInspector;
        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let call_size = 36usize;
        let num_calls = std::cmp::max(1, input.len() / call_size);
        let num_calls = std::cmp::min(num_calls, 5);
        let mut nonce = start_nonce;

        for i in 0..num_calls {
            let start = i * call_size;
            let end = std::cmp::min(start + call_size, input.len());
            let call_data = &input[start..end];

            let tx = TxEnv {
                caller: CALLER,
                kind: TxKind::Call(self.contract_address),
                data: Bytes::copy_from_slice(call_data),
                gas_limit: GAS_LIMIT,
                nonce,
                ..Default::default()
            };

            let result = evm.inspect_tx_commit(tx)?;
            nonce += 1;
            if !result.is_success() {
                return Ok(false); // reverted
            }
        }

        Ok(true)
    }
}
