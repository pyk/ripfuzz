//! Ripfuzz VM contract address.

use alloy_primitives::{Address, address};

/// Cheat code address.
///
/// Calculated as `address(uint160(uint256(keccak256("ripfuzz cheatcode"))))`.
pub const VM_ADDRESS: Address = address!("0x628dC59F11F72B611132eC40437F125ba1312F08");

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::keccak256;

    #[test]
    fn vm_address_matches_ripfuzz_cheatcode_string() {
        let hash = keccak256(b"ripfuzz cheatcode");
        let expected = Address::from_word(hash);
        assert_eq!(expected, VM_ADDRESS);
    }
}
