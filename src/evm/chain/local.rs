//! Local sandbox chain initialisation.

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
use crate::evm::database::{Database, LocalDB};
use crate::vm::VM_ADDRESS;

impl Default for Chain {
    fn default() -> Self {
        Self::local()
    }
}

impl Chain {
    /// Create a new local sandbox EVM.
    pub fn local() -> Self {
        let mut cfg_env = CfgEnv::default();
        cfg_env.chain_id = 1;
        cfg_env.tx_gas_limit_cap = Some(u64::MAX);
        cfg_env.disable_nonce_check = true;
        cfg_env.disable_eip3607 = true;
        cfg_env.disable_base_fee = true;
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

        let mut db = CacheDB::new(LocalDB::default());
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
            database: Some(Database::Local(db)),
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

    use crate::evm::chain::{Chain, DEFAULT_DEPLOYER};
    use crate::vm::VM_ADDRESS;

    #[test]
    fn chain_new_uses_latest_spec() {
        let chain = Chain::local();
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
        let chain = Chain::local();
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
        let mut chain = Chain::local();

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

        let (deployed_address, _) = chain
            .deploy(DEFAULT_DEPLOYER, U256::ZERO, initcode)
            .unwrap();

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
        let chain = Chain::local();
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
    /// for never-seen addresses.  If `Database::basic` returns `None`,
    /// revm's `CacheDB` marks the account as `AccountState::NotExisting`.
    /// A sandbox has no state trie, so there is no concept of "non-existing"
    /// vs "empty"; every address must be treated as empty.
    #[test]
    fn chain_new_returns_default_account_info_for_unknown_address() {
        let mut chain = Chain::local();
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
        let chain = Chain::local();
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
        let mut chain = Chain::local();

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

        let result = chain.deploy(DEFAULT_DEPLOYER, U256::ZERO, Bytes::from(initcode));
        assert!(
            result.is_ok(),
            "Chain::new must disable code size limit so contracts > 32 KB can deploy"
        );

        let (address, tx_result) = result.unwrap();
        assert!(tx_result.success, "large deployment must succeed");
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
}
