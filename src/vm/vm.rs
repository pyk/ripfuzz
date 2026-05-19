//! VM component instance: cheatcode dispatch.

use revm::primitives::Bytes;

use crate::vm::{CheatcodeEffect, VmConfig, dispatch_effects};

/// Component instance for the VM layer.
#[derive(Debug, Clone)]
pub struct Vm {
    config: VmConfig,
}

impl Vm {
    pub fn new(config: VmConfig) -> Self {
        Self { config }
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
}
