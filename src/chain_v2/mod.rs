//! Chain abstraction v2: clean, revm-native, decoupled from fuzzing logic.
//!
//! Lifecycle:
//! 1. Resolve environment (local or fork).
//! 2. Create chain (build database + genesis).
//! 3. Deploy target contract.
//! 4. Execute setup function.
//!
//! The chain owns the post-setup state; the fuzzer clones it via [`StateSnapshot`].

use alloy_primitives::Address;
use anyhow::Result;
use revm::primitives::{Bytes, U256};

pub use database::Database;
pub use deploy::execute as deploy;
pub use environment::{BlockHeader, Environment};
pub use error::{DeployError, SetupError};
pub use setup::execute as setup;
pub use state::StateSnapshot;
pub use trace::{CallFrame, Trace};

use crate::target::Contract;

pub mod database;
pub mod deploy;
pub mod environment;
pub mod error;
pub mod setup;
pub mod state;
pub mod trace;

/// Default deployer address used when none is specified.
pub const DEFAULT_DEPLOYER: Address = Address::new([
    0xc3, 0x42, 0x96, 0x17, 0x5b, 0x9e, 0x78, 0xf6, 0x6e, 0xdb, 0xea, 0xeb, 0x7a, 0xce, 0xa4, 0xc6,
    0x15, 0xc0, 0x92, 0xe1,
]);

/// Chain after environment resolution and optional genesis allocations.
#[derive(Clone, Debug)]
pub struct Chain {
    pub state: StateSnapshot,
    pub contract_address: Option<Address>,
    deploy_value: U256,
}

impl Chain {
    /// Initialize a new chain from the resolved environment.
    pub fn new(env: &Environment) -> Result<Self> {
        let (db, block_env, cfg_env) = env.clone().into_components()?;
        let mut state = StateSnapshot {
            db,
            block_env,
            cfg_env,
            deployer: DEFAULT_DEPLOYER,
            deployer_nonce: 0,
        };
        state.seed_deployer(DEFAULT_DEPLOYER);

        Ok(Self {
            state,
            contract_address: None,
            deploy_value: U256::ZERO,
        })
    }

    /// Override the deployer address.
    pub fn with_deployer(mut self, deployer: Address) -> Self {
        if self.state.deployer != deployer {
            self.state.seed_deployer(deployer);
        }
        self
    }

    /// Set the wei value sent during deployment.
    pub fn with_deploy_value(mut self, value: U256) -> Self {
        self.deploy_value = value;
        self
    }

    /// Pre-deploy a contract at the given address.
    pub fn with_predeploy(mut self, address: Address, code: Bytes) -> Self {
        self.state.predeploy(address, code);
        self
    }

    /// Deploy the target contract.
    pub fn deploy(mut self, target: &Contract) -> Result<Self, DeployError> {
        let (address, state) = deploy::execute(self.state, target, self.deploy_value)?;
        self.state = state;
        self.contract_address = Some(address);
        Ok(self)
    }

    /// Run the target contract's optional `setup()` function.
    pub fn setup(mut self, target: &Contract) -> Result<Self, SetupError> {
        let state = setup::execute(self.state, self.contract_address, target)?;
        self.state = state;
        Ok(self)
    }

    /// Take a cloneable snapshot of the current chain state.
    pub fn snapshot(&self) -> StateSnapshot {
        self.state.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundry;
    use crate::target::Contract;

    fn load_target(project: &str, id: &str) -> Contract {
        let project = foundry::Project::new(project);
        let artifacts = project.load_artifacts().unwrap();
        let artifact_id = foundry::ArtifactId::try_from(id).unwrap();
        let artifact = artifacts.get(&artifact_id).unwrap();
        Contract::try_from(artifact).unwrap()
    }

    #[test]
    fn chain_v2_deploy_and_setup_local() {
        let target = load_target(
            "fixtures/basic-target",
            "src/NamedMismatch.sol:DifferentName",
        );
        let env = Environment::local();
        let chain = Chain::new(&env)
            .unwrap()
            .deploy(&target)
            .unwrap()
            .setup(&target)
            .unwrap();

        assert!(chain.contract_address.is_some());
    }

    #[test]
    fn chain_v2_deploy_with_setup_contract() {
        let target = load_target(
            "fixtures/target-contract-validation",
            "src/ValidSetup.sol:ValidSetup",
        );
        let env = Environment::local();
        let chain = Chain::new(&env)
            .unwrap()
            .deploy(&target)
            .unwrap()
            .setup(&target)
            .unwrap();

        assert!(chain.contract_address.is_some());
    }
}
