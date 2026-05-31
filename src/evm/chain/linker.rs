//! Solidity library linker.

use std::collections::HashMap;

use alloy_primitives::Address;

/// Linker operation type that replaces Solidity library placeholders in
/// initcode with deployed addresses.
pub struct Linker;

impl Linker {
    /// Compute the Solidity placeholder string for a library identifier.
    ///
    /// The placeholder format is `__$<keccak256(identifier)[:34]>$__`.
    pub fn get_library_placeholder(identifier: &str) -> String {
        let hash = alloy_primitives::keccak256(identifier.as_bytes());
        let hex = alloy_primitives::hex::encode(hash);
        format!("__${}$__", &hex[..34])
    }

    /// Replace library placeholders in initcode with deployed addresses.
    pub fn link_libraries(initcode: &str, libraries: &HashMap<String, Address>) -> String {
        let mut hex = initcode.to_owned();
        for (identifier, address) in libraries {
            let placeholder = Self::get_library_placeholder(identifier);
            let address_hex = hex::encode(address);
            hex = hex.replace(&placeholder, &address_hex);
        }
        hex
    }
}
