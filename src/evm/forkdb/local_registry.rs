//! Registry of locally-created addresses shared across EVM inspectors
//! and the ForkDB backend.
//!
//! During fork mode execution, the EVM creates addresses through two
//! distinct paths:
//!
//! 1. CREATE / CREATE2 opcodes -- tracked by [`LocalTracker`].
//! 2. `vm.addr` cheatcode -- tracked by [`cheatcode::Inspector`].
//!
//! Both paths mark addresses in this shared registry. The ForkDB
//! backend then skips RPC fetches for any address present here.

use std::collections::HashSet;
use std::sync::Arc;

use alloy_primitives::Address;
use parking_lot::RwLock;

/// Inner state shared across all clones.
#[derive(Debug, Default)]
struct SharedLocalAddressRegistryInner {
    addresses: RwLock<HashSet<Address>>,
}

/// Thread-safe registry of locally-created addresses.
///
/// Cloning is cheap (shares the same inner state). Used by
/// [`LocalTracker`], the cheatcode inspector, and the ForkDB backend
/// to avoid unnecessary RPC fetches for addresses that only exist
/// inside the local EVM.
#[derive(Debug, Clone, Default)]
pub struct SharedLocalAddressRegistry {
    inner: Arc<SharedLocalAddressRegistryInner>,
}

impl SharedLocalAddressRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(SharedLocalAddressRegistryInner::default()),
        }
    }

    /// Mark an address as locally-created.
    ///
    /// Thread-safe: can be called concurrently from EVM inspectors
    /// during execution.
    pub fn mark_local(&self, address: Address) {
        self.inner.addresses.write().insert(address);
    }

    /// Check whether an address is locally-created (no RPC needed).
    pub fn is_local(&self, address: Address) -> bool {
        self.inner.addresses.read().contains(&address)
    }
}
