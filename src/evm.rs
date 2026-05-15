use revm::{
    context::{Context, TxEnv},
    database::InMemoryDB,
    database_interface::Database,
    handler::ExecuteCommitEvm,
    inspector::InspectCommitEvm,
    primitives::{Address, Bytes, TxKind, U256, KECCAK_EMPTY},
    state::AccountInfo,
    MainBuilder, MainContext,
};

use crate::foundry::FoundryArtifact;
use crate::inspector::CoverageInspector;

pub const CALLER: Address = Address::new([0xde; 20]);
pub const GAS_LIMIT: u64 = 1_000_000;

pub struct EvmRunner {
    pub contract_address: Address,
    pub deployed_db: InMemoryDB,
}

impl EvmRunner {
    pub fn deploy(artifact: &FoundryArtifact) -> anyhow::Result<Self> {
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

        let ctx = Context::mainnet().with_db(db);
        let mut evm = ctx.build_mainnet();

        let creation_bytecode = artifact.creation_bytecode()?;
        let tx = TxEnv {
            caller: CALLER,
            kind: TxKind::Create,
            data: Bytes::from(creation_bytecode),
            gas_limit: GAS_LIMIT,
            ..Default::default()
        };

        let result = evm.transact_commit(tx)?;
        let contract_address = result
            .created_address()
            .ok_or_else(|| anyhow::anyhow!("deployment failed"))?;

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
