//! Raptor VM contract address.

use revm::primitives::Address;

/// Raptor VM contract address.
///
/// Derived from `address(uint160(uint256(keccak256("raptor vm"))))`.
///
/// NOTE: The raptor VM is **not** Foundry VM compatible. It does not
/// implement all Foundry cheatcodes - only the subset documented in the
/// raptor cheatcode module.
pub const VM_ADDRESS: Address = Address::new([
    0x26, 0x3a, 0xf5, 0x13, 0xa0, 0x43, 0x5e, 0xbc, 0x9d, 0x5c, 0x36, 0x2c, 0xf7, 0x62, 0x52, 0xf8,
    0x71, 0x73, 0xf8, 0xf1,
]);

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::keccak256;

    #[test]
    fn vm_address_matches_raptor_vm_string() {
        let hash = keccak256(b"raptor vm");
        let expected = Address::from_word(hash);
        assert_eq!(expected, VM_ADDRESS);
    }
}
