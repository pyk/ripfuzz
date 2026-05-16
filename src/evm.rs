//! EVM runner for deploying and executing Solidity contracts.

use std::collections::HashMap;

use alloy_dyn_abi::DynSolType;
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::JsonAbi;
use anyhow::Result;
use revm::{
    InspectEvm, MainBuilder, MainContext,
    context::{Context, TxEnv, result::ExecutionResult},
    database::InMemoryDB,
    database_interface::Database,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, KECCAK_EMPTY, TxKind, U256},
    state::AccountInfo,
};

use crate::contract;
use crate::fuzzer::sequence;
use crate::inspector;
use crate::trace;

pub const CALLER: Address = Address::new([0xde; 20]);
pub const GAS_LIMIT: u64 = 1_000_000;

/// Extract a human-readable error message from a failed deployment result.
fn extract_deployment_error(result: &ExecutionResult) -> String {
    match result {
        ExecutionResult::Success { .. } => "contract returned no address".into(),
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
    let decoded = crate::result_to_option(string_type.abi_decode_params(&output[4..]))?;

    match decoded {
        DynSolValue::String(s) => Some(s),
        _ => None,
    }
}

/// Solidity `Error(string)` selector: `keccak256("Error(string)")[:4]`
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

/// Metadata for a single call in an executed sequence.
#[derive(Debug, Clone)]
pub struct CallMeta {
    /// Block number at execution time.
    pub block_number: u64,
    /// Block timestamp at execution time.
    pub block_timestamp: u64,
}

/// The result of a sequence execution.
pub struct SequenceResult {
    /// Whether every call in the sequence succeeded (no reverts).
    pub all_ok: bool,
    /// Whether at least one property returned `true` after the sequence.
    pub property_triggered: bool,
    /// Name of the triggered property, if any.
    pub triggered_property: Option<String>,
    /// Selector of the triggered property, if any.
    pub triggered_property_selector: Option<[u8; 4]>,
    /// Per-call execution metadata (block number / timestamp).
    pub call_meta: Vec<CallMeta>,
}

#[derive(Debug)]
pub struct EvmRunner {
    pub contract_address: Address,
    pub deployed_db: InMemoryDB,
    pub properties: Vec<([u8; 4], String)>,
    pub contract_name: String,
    pub contract_abi: JsonAbi,
    pub initcode_map: HashMap<Bytes, (String, JsonAbi)>,
}

impl EvmRunner {
    pub fn from_target(target: &contract::ContractArtifact) -> Result<Self> {
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

        let inspector = trace::CallTraceInspector::new(target.initcode_map.clone());
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
        let has_setup = target
            .abi
            .functions()
            .any(|f| f.selector() == SETUP_SELECTOR);
        if has_setup {
            let nonce = crate::result_to_option(evm.ctx.journaled_state.database.basic(CALLER))
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
            contract_name: target.contract_name.clone(),
            contract_abi: target.abi.clone(),
            initcode_map: target.initcode_map.clone(),
        })
    }

    pub fn run_sequence(
        &self,
        calls: &[sequence::Call],
        inspector: inspector::CoverageInspector,
    ) -> Result<SequenceResult, anyhow::Error> {
        let mut db = self.deployed_db.clone();
        let start_nonce = db
            .basic(CALLER)
            .map_err(|_| anyhow::anyhow!("db error"))?
            .unwrap_or_default()
            .nonce;

        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet_with_inspector(inspector);

        let mut nonce = start_nonce;
        let mut call_meta = Vec::new();

        for (idx, call) in calls.iter().enumerate().take(5) {
            // Apply per-call block delays before execution.
            let number_delay = U256::from(call.block_number_delay);
            let time_delay = U256::from(call.block_timestamp_delay);
            if idx > 0 {
                // Raptor commits every transaction immediately and cannot pack
                // multiple calls into the same block. Ensure each subsequent call
                // gets a distinct block context even when the delay is 0.
                evm.ctx.block.number += number_delay.max(U256::from(1));
                evm.ctx.block.timestamp += time_delay.max(U256::from(1));
            } else {
                evm.ctx.block.number += number_delay;
                evm.ctx.block.timestamp += time_delay;
            }

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

            call_meta.push(CallMeta {
                block_number: evm.ctx.block.number.try_into().unwrap_or(0),
                block_timestamp: evm.ctx.block.timestamp.try_into().unwrap_or(0),
            });

            if !result.is_success() {
                return Ok(SequenceResult {
                    all_ok: false,
                    property_triggered: false,
                    triggered_property: None,
                    triggered_property_selector: None,
                    call_meta,
                });
            }
        }

        // After a successful sequence, check whether any property returns `true`.
        let mut triggered_property = None;
        let mut triggered_property_selector = None;
        for (selector, name) in &self.properties {
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
                    triggered_property = Some(name.to_owned());
                    triggered_property_selector = Some(*selector);
                    break;
                }
            }
        }

        Ok(SequenceResult {
            all_ok: true,
            property_triggered: triggered_property.is_some(),
            triggered_property,
            triggered_property_selector,
            call_meta,
        })
    }
}
