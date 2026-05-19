//! VM component instance: cheatcode dispatch and state factory.

use revm::primitives::Bytes;

use crate::vm::inspector::CheatcodeInspector;
use crate::vm::{CheatcodeEffect, VmConfig, VmState, dispatch_effects};

/// Component instance for the VM layer.
#[derive(Debug, Clone)]
pub struct Vm {
    config: VmConfig,
}

impl Vm {
    pub fn new(config: VmConfig) -> Self {
        Self { config }
    }

    /// Produce a fresh VmState for a new chain snapshot.
    pub fn fresh_state(&self) -> VmState {
        VmState {
            ffi_enabled: self.config.ffi,
            project_root: self.config.project_root.clone(),
            ..VmState::default()
        }
    }

    /// Build a CheatcodeInspector from a given VmState.
    pub fn inspector(&self, state: VmState) -> CheatcodeInspector {
        CheatcodeInspector::from_state(state)
    }

    /// Resolve a cheatcode selector to its effects.
    pub fn dispatch(&self, selector: [u8; 4], input: &[u8]) -> Option<Vec<CheatcodeEffect>> {
        dispatch_effects(selector, &Bytes::from(input.to_vec()))
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }
}

impl crate::vm::VmFactory for Vm {
    fn config(&self) -> &VmConfig {
        &self.config
    }

    fn fresh_state(&self) -> VmState {
        self.fresh_state()
    }
}
