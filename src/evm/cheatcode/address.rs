//! Raptor VM contract address.

use alloy_primitives::{Address, address};

/// Cheat code address.
///
/// Calculated as `address(uint160(uint256(keccak256("hevm cheat code"))))`.
///
/// This is the same address used by Foundry, ensuring compatibility with
/// existing contracts that reference `vm` at this hardcoded address.
pub const VM_ADDRESS: Address = address!("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D");

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::utils::keccak256;

    #[test]
    fn vm_address_matches_hevm_cheat_code_string() {
        let hash = keccak256(b"hevm cheat code");
        let expected = Address::from_word(hash);
        assert_eq!(expected, VM_ADDRESS);
    }
}
