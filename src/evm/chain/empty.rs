//! Empty sandbox chain initialisation.

use alloy_primitives::{Address, B256, U256};
use revm::{
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv},
    context_interface::block::BlobExcessGasAndPrice,
    database::CacheDB,
    primitives::Bytes,
    primitives::hardfork::SpecId,
    state::AccountInfo,
};

use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
use crate::evm::cheatcode::VM_ADDRESS;
use crate::evm::database::{Database, EmptyDB};

impl Default for Chain {
    fn default() -> Self {
        Self::empty()
    }
}

impl Chain {
    /// Create a new empty sandbox EVM.
    pub fn empty() -> Self {
        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = 1;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.disable_base_fee = true;
        cfg_env.tx_chain_id_check = false;
        cfg_env.limit_contract_code_size = Some(usize::MAX);
        cfg_env.limit_contract_initcode_size = Some(usize::MAX);
        cfg_env.set_spec_and_mainnet_gas_params(SpecId::AMSTERDAM);

        let mut block_env = BlockEnv {
            number: U256::from(1),
            beneficiary: Address::ZERO,
            timestamp: U256::from(1_438_269_988_u64),
            gas_limit: u64::MAX,
            basefee: 0,
            difficulty: U256::ZERO,
            prevrandao: Some(B256::ZERO),
            blob_excess_gas_and_price: None,
            slot_num: 0,
        };

        // NOTE: This is required for post-Cancun
        block_env.blob_excess_gas_and_price =
            Some(BlobExcessGasAndPrice::new_with_spec(0, SpecId::AMSTERDAM));

        let mut db = CacheDB::new(EmptyDB::default());
        let info = AccountInfo {
            balance: U256::MAX,
            nonce: 0,
            code_hash: revm::primitives::KECCAK_EMPTY,
            code: None,
            account_id: None,
        };
        db.insert_account_info(DEFAULT_DEPLOYER, info);

        // Insert a dummy VM contract so Solidity's `extcodesize` check passes
        // when a target calls raptor cheatcodes during deployment or setup.
        let vm_code = Bytecode::new_raw(Bytes::from_static(&[0x00]));
        db.insert_account_info(
            VM_ADDRESS,
            AccountInfo {
                balance: U256::ZERO,
                nonce: 0,
                code_hash: vm_code.hash_slow(),
                code: Some(vm_code),
                account_id: None,
            },
        );

        Self {
            database: Some(Database::Empty(db)),
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, U256, address};
    use revm::Database;
    use revm::DatabaseRef;
    use revm::bytecode::opcode::{CODECOPY, MSTORE, PUSH1, PUSH2, RETURN};
    use revm::primitives::Bytes;
    use revm::primitives::hardfork::SpecId;

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER, DeployInput};
    use crate::evm::cheatcode::VM_ADDRESS;

    #[test]
    fn chain_new_uses_latest_spec() {
        let chain = Chain::empty();
        assert_eq!(
            chain.cfg_env().spec,
            SpecId::AMSTERDAM,
            "Chain::new should use latest spec (AMSTERDAM)"
        );
    }

    #[test]
    fn default_deployer_matches_raptor_deployer_string() {
        let hash = alloy_primitives::utils::keccak256(b"raptor deployer");
        let expected = revm::primitives::Address::from_word(hash);
        assert_eq!(expected, DEFAULT_DEPLOYER);
    }

    #[test]
    fn chain_new_seeds_deployer_with_max_balance() {
        let chain = Chain::empty();
        assert_eq!(
            chain.deployer(),
            DEFAULT_DEPLOYER,
            "deployer should default to DEFAULT_DEPLOYER"
        );
        let db = chain.database().unwrap();
        let info = db.basic_ref(DEFAULT_DEPLOYER).unwrap();
        let balance = info.map(|i| i.balance).unwrap_or_default();
        assert_eq!(
            balance,
            U256::MAX,
            "deployer must be seeded with U256::MAX in Chain::new"
        );
    }

    #[test]
    fn chain_new_allows_contract_as_caller() {
        let mut chain = Chain::empty();

        // Initcode that returns 1 byte of runtime code (0x00 STOP) so the
        // deployed address has non-empty code.
        let initcode = Bytes::from_static(&[
            PUSH1, 0x01, // PUSH1 1
            PUSH1, 0x00,   // PUSH1 0
            MSTORE, // MSTORE
            PUSH1, 0x01, // PUSH1 1
            PUSH1, 0x00,   // PUSH1 0
            RETURN, // RETURN
        ]);

        let opts = DeployInput::new(initcode);
        let deployment = chain.deploy(opts).unwrap();
        let deployed_address = deployment.address.unwrap();

        // Calling from a contract address should succeed when EIP-3607 is disabled.
        let result = chain.call(deployed_address, Address::ZERO, U256::ZERO, Bytes::new());
        assert!(
            result.is_ok(),
            "EIP-3607 must be disabled so a contract can act as caller"
        );
    }

    /// Chain::new must inject a dummy contract at the raptor VM address so
    /// that Solidity `extcodesize` checks do not revert when a target contract
    /// calls cheatcodes during deployment or setup.
    #[test]
    fn chain_new_injects_vm_address() {
        let chain = Chain::empty();
        let db = chain.database().unwrap();
        let info = db.basic_ref(VM_ADDRESS).unwrap();
        let info = info.unwrap();
        let code = info.code.as_ref().unwrap();
        assert!(
            !code.is_empty(),
            "Chain::new must inject non-empty code at VM_ADDRESS so extcodesize checks pass"
        );
    }

    /// Chain::new must use a database that returns `Some(AccountInfo::default())`
    /// for never-seen addresses. If `Database::basic` returns `None`,
    /// revm's `CacheDB` marks the account as `AccountState::NotExisting`.
    /// A sandbox has no state trie, so there is no concept of "non-existing"
    /// vs "empty"; every address must be treated as empty.
    #[test]
    fn chain_new_returns_default_account_info_for_unknown_address() {
        let mut chain = Chain::empty();
        let db = chain.database_mut().expect("database should be available");
        let unknown = address!("0x00000000000000000000000000000000000000ab");
        let info = db.basic(unknown).unwrap();
        assert!(
            info.is_some(),
            "a sandbox database must return Some(AccountInfo::default()) for every address; \
             got None, which marks the account as NotExisting in CacheDB"
        );
        assert_eq!(info.unwrap(), AccountInfo::default());
    }

    /// Chain::new must use the Ethereum mainnet block 1 timestamp
    /// (`1438269988`) instead of a small sentinel like 1, which predates the
    /// Unix epoch and can trigger underflows in contracts that compare
    /// `block.timestamp` against deployment time or constant offsets.
    #[test]
    fn chain_new_uses_mainnet_block_one_timestamp() {
        let chain = Chain::empty();
        assert_eq!(
            chain.block_env().timestamp,
            U256::from(1_438_269_988_u64),
            "Chain::new should use the mainnet block 1 timestamp (1438269988)"
        );
    }

    /// Chain::new must disable the contract code size limit so
    /// that large factory contracts or inlined targets can deploy.
    #[test]
    fn chain_new_allows_unlimited_contract_size() {
        let mut chain = Chain::empty();

        // Build initcode that returns 0x8001 bytes (32769) of runtime code,
        // which is one byte larger than the EIP-7954 limit of 0x8000 (32768)
        // enforced for the AMSTERDAM spec.
        //
        // Initcode:
        //   PUSH2 0x8001       // size to copy
        //   PUSH1 0x0e         // offset in this initcode to padding
        //   PUSH1 0x00         // dest offset in memory
        //   CODECOPY           // copy padding into memory
        //   PUSH2 0x8001       // size to return
        //   PUSH1 0x00         // mem offset
        //   RETURN             // return memory as runtime code
        let mut initcode = vec![
            PUSH2, 0x80, 0x01, // PUSH2 0x8001
            PUSH1, 0x0e, // PUSH1 0x0e
            PUSH1, 0x00,     // PUSH1 0x00
            CODECOPY, // CODECOPY
            PUSH2, 0x80, 0x01, // PUSH2 0x8001
            PUSH1, 0x00,   // PUSH1 0x00
            RETURN, // RETURN
        ];
        initcode.extend(std::iter::repeat(0x00).take(0x8001));

        let opts = DeployInput::new(Bytes::from(initcode));
        let deployment = chain.deploy(opts).unwrap();
        assert!(deployment.result.success, "large deployment must succeed");
        let address = deployment.address.unwrap();
        assert_ne!(
            address,
            Address::ZERO,
            "must return a valid deployed address"
        );

        // Verify the deployed bytecode is actually 32769 bytes.
        let db = chain.database().expect("database should be available");
        let info = db
            .basic_ref(address)
            .unwrap()
            .expect("account should exist");
        let code_len = info.code.map(|c| c.len()).unwrap_or(0);
        assert_eq!(code_len, 0x8001, "deployed code must be 32769 bytes");
    }

    /// Load a [`target::Contract`](crate::target::Contract) from a pre-built
    /// fixture by its full artifact id (`path:name`).
    fn load_fixture(id: &str) -> crate::target::Contract {
        let project = crate::foundry::Project::new("fixtures/target-contract-deployment");
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = crate::foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        crate::target::Contract::try_from(artifact).unwrap()
    }

    /// A target contract with a constructor but no `setup()` function must
    /// deploy successfully on an empty sandbox chain.
    #[test]
    fn deploy_no_setup_succeeds() {
        let contract = load_fixture("src/EmptyChainNoSetup.sol:EmptyChainNoSetup");

        let mut chain = Chain::empty();
        let opts = DeployInput::new(contract.initcode);
        let deployment = chain.deploy(opts).unwrap();

        assert!(deployment.result.success, "deployment must succeed");
        let address = deployment.address.unwrap();

        let db = chain.database().expect("database should be available");
        let info = db
            .basic_ref(address)
            .unwrap()
            .expect("account should exist after deployment");
        assert!(
            info.code.as_ref().map(|c| !c.is_empty()).unwrap_or(false),
            "deployed contract must have non-empty runtime code"
        );
    }

    /// A target contract whose constructor reverts must fail deployment on an
    /// empty sandbox chain.
    #[test]
    fn deploy_constructor_revert_fails() {
        let contract =
            load_fixture("src/EmptyChainConstructorRevert.sol:EmptyChainConstructorRevert");

        let mut chain = Chain::empty();
        let opts = DeployInput::new(contract.initcode);
        let deployment = chain.deploy(opts).unwrap();

        assert!(
            !deployment.result.success,
            "deployment must fail when constructor reverts"
        );
        assert!(
            deployment.address.is_none(),
            "no address must be created on revert"
        );
        assert_eq!(
            deployment.trace.roots.len(),
            1,
            "trace must contain the root create frame"
        );
    }

    /// A target contract that calls a raptor cheatcode in its constructor must
    /// deploy successfully on an empty sandbox chain.
    #[test]
    fn deploy_cheatcode_in_constructor_succeeds() {
        let contract = load_fixture(
            "src/EmptyChainCheatcodeInConstructor.sol:EmptyChainCheatcodeInConstructor",
        );

        let mut chain = Chain::empty();
        let opts = DeployInput::new(contract.initcode);
        let deployment = chain.deploy(opts).unwrap();

        assert!(
            deployment.result.success,
            "deployment must succeed when cheatcode works in constructor"
        );
        let address = deployment.address.unwrap();

        let db = chain.database().expect("database should be available");
        let info = db
            .basic_ref(address)
            .unwrap()
            .expect("account should exist after deployment");
        assert!(
            info.code.as_ref().map(|c| !c.is_empty()).unwrap_or(false),
            "deployed contract must have non-empty runtime code"
        );
    }

    /// A target contract with no constructor but a `setup()` that calls a
    /// raptor cheatcode must deploy and setup successfully on an empty sandbox
    /// chain.
    #[test]
    fn setup_cheatcode_succeeds() {
        let contract =
            load_fixture("src/EmptyChainCheatcodeInSetup.sol:EmptyChainCheatcodeInSetup");

        let mut chain = Chain::empty();
        let deploy_opts = DeployInput::new(contract.initcode);
        let deployment = chain.deploy(deploy_opts).unwrap();

        assert!(
            deployment.result.success,
            "deployment must succeed for contract with setup-only cheatcode"
        );
        let address = deployment.address.unwrap();

        let setup_func = contract
            .setup_function
            .as_ref()
            .expect("setup function must exist in ABI");
        let setup_data = Bytes::from(setup_func.selector().as_slice().to_vec());
        let setup_opts = crate::evm::chain::SetupInput::new(address).calldata(setup_data);
        let setup = chain.setup(setup_opts).unwrap();

        assert!(
            setup.result.success,
            "setup must succeed when cheatcode works in setup"
        );
    }
}
