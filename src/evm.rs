use std::collections::HashMap;

use alloy_dyn_abi::DynSolType;
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::JsonAbi;
use anyhow::Result;
use revm::{
    InspectEvm,
    MainBuilder, MainContext,
    context::{Context, TxEnv},
    database::InMemoryDB,
    database_interface::Database,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
    state::AccountInfo,
};

use crate::contract::ContractArtifact;
use crate::fuzzer::sequence::Call;
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

/// The result of a sequence execution.
pub struct SequenceResult {
    /// Whether every call in the sequence succeeded (no reverts).
    pub all_ok: bool,
    /// Whether at least one property returned `true` after the sequence.
    pub property_triggered: bool,
}

#[derive(Debug)]
pub struct EvmRunner {
    pub contract_address: Address,
    pub deployed_db: InMemoryDB,
    pub properties: Vec<([u8; 4], String)>,
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

        // If the contract declares a `setUp()` function, call it after deployment.
        const SETUP_SELECTOR: [u8; 4] = [0x0a, 0x92, 0x54, 0xe4];
        let has_setup = target.abi.functions().any(|f| f.selector() == SETUP_SELECTOR);
        if has_setup {
            let nonce = evm
                .ctx
                .journaled_state
                .database
                .basic(CALLER)
                .ok()
                .flatten()
                .map(|info| info.nonce)
                .unwrap_or(0);
            let setup_tx = TxEnv {
                caller: CALLER,
                kind: TxKind::Call(contract_address),
                data: Bytes::copy_from_slice(&SETUP_SELECTOR),
                gas_limit: GAS_LIMIT,
                nonce,
                ..Default::default()
            };
            let setup_result = evm.inspect_tx_commit(setup_tx)?;
            if !setup_result.is_success() {
                let reason = extract_deployment_error(&setup_result);
                let trace = evm.inspector.format();
                return Err(anyhow::anyhow!("setUp failed: {reason}\n\nTrace:\n{trace}"));
            }
        }

        let deployed_db = evm.ctx.journaled_state.database;
        Ok(Self {
            contract_address,
            deployed_db,
            properties: target.properties.clone(),
        })
    }

    pub fn run_sequence(&self, calls: &[Call]) -> Result<SequenceResult, anyhow::Error> {
        let mut db = self.deployed_db.clone();
        let start_nonce = db
            .basic(CALLER)
            .map_err(|_| anyhow::anyhow!("db error"))?
            .unwrap_or_default()
            .nonce;

        let inspector = CoverageInspector;
        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let mut nonce = start_nonce;

        for call in calls.iter().take(5) {
            let mut call_data = Vec::with_capacity(call.encoded_size());
            call_data.extend_from_slice(&call.selector);
            call_data.extend_from_slice(&call.args);

            let tx = TxEnv {
                caller: CALLER,
                kind: TxKind::Call(self.contract_address),
                data: Bytes::from(call_data),
                gas_limit: GAS_LIMIT,
                nonce,
                ..Default::default()
            };

            let result = evm.inspect_tx_commit(tx)?;
            nonce += 1;
            if !result.is_success() {
                return Ok(SequenceResult {
                    all_ok: false,
                    property_triggered: false,
                });
            }
        }

        // After a successful sequence, check whether any property returns `true`.
        let mut property_triggered = false;
        for (selector, _name) in &self.properties {
            let tx = TxEnv {
                caller: CALLER,
                kind: TxKind::Call(self.contract_address),
                data: Bytes::copy_from_slice(selector),
                gas_limit: GAS_LIMIT,
                nonce,
                ..Default::default()
            };
            let result = evm.inspect_one_tx(tx)?;
            if result.is_success() {
                let out: Option<&Bytes> = result.output();
                if let Some(output) = out
                    && output.len() == 32
                    && output[31] == 1
                {
                    property_triggered = true;
                    break;
                }
            }
        }

        Ok(SequenceResult {
            all_ok: true,
            property_triggered,
        })
    }
}
