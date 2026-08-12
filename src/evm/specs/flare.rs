//! Flare-family network hardfork schedules.
//!
//! Flare chains are EVM-compatible (standard Ethereum JSON-RPC and
//! EIP-2718 transactions) but are absent from the upstream
//! `alloy-hardforks` tables, so their activations are resolved here.
//!
//! Timestamps are the network upgrade activations from
//! [go-flare's `upgrade.go`](https://github.com/flare-foundation/go-flare/blob/main/avalanchego/upgrade/upgrade.go):
//! - Durango activates the Shanghai execution spec (EIP-4895 withdrawals are
//!   not enabled on Flare)
//! - Etna activates Cancun (EIP-4844 blob transactions are not enabled)
//!
//! Before Durango the C-Chain runs the London spec (Apricot Phase 3; London
//! from genesis on Flare and Coston2, activated at blocks 12349716 and 55188
//! on Songbird and Coston in early 2022). Later upgrades (Fortuna, Granite)
//! are Avalanche-specific and do not change EVM opcode or gas behaviour.

use revm::primitives::hardfork::SpecId;

/// (chain_id, shanghai_time, cancun_time) activation timestamps.
const ACTIVATIONS: [(u64, u64, u64); 4] = [
    (14, 1_754_395_200, 1_764_676_800),  // Flare mainnet
    (19, 1_753_185_600, 1_764_072_000),  // Songbird
    (114, 1_750_766_400, 1_763_042_400), // Coston2
    (16, 1_751_371_200, 1_763_028_000),  // Coston
];

/// Resolve the [`SpecId`] for a Flare-family chain at the given block
/// timestamp. Returns `None` for chain ids outside the family.
pub fn spec_id(chain_id: u64, timestamp: u64) -> Option<SpecId> {
    let (_, shanghai, cancun) = ACTIVATIONS.iter().find(|(id, _, _)| *id == chain_id)?;
    Some(match timestamp {
        _ if timestamp < *shanghai => SpecId::LONDON,
        _ if timestamp < *cancun => SpecId::SHANGHAI,
        _ => SpecId::CANCUN,
    })
}

#[cfg(test)]
mod tests {
    use revm::primitives::hardfork::SpecId;

    use crate::evm::specs::get_spec_id;

    #[test]
    fn flare_mainnet_pre_durango_is_london() {
        // Durango (Shanghai) activated 2025-08-05 12:00 UTC
        assert_eq!(get_spec_id(14, 1_754_395_199), SpecId::LONDON);
    }

    #[test]
    fn flare_mainnet_between_durango_and_etna_is_shanghai() {
        // Etna (Cancun) activated 2025-12-02 12:00 UTC
        assert_eq!(get_spec_id(14, 1_754_395_200), SpecId::SHANGHAI);
        assert_eq!(get_spec_id(14, 1_764_676_799), SpecId::SHANGHAI);
    }

    #[test]
    fn flare_mainnet_post_etna_is_cancun() {
        assert_eq!(get_spec_id(14, 1_764_676_800), SpecId::CANCUN);
        // Recent block (2026-08-12), after the Avalanche-only Granite upgrade
        assert_eq!(get_spec_id(14, 1_786_492_800), SpecId::CANCUN);
    }

    #[test]
    fn flare_family_recent_blocks_are_cancun() {
        assert_eq!(get_spec_id(19, 1_786_492_800), SpecId::CANCUN); // Songbird
        assert_eq!(get_spec_id(114, 1_786_492_800), SpecId::CANCUN); // Coston2
        assert_eq!(get_spec_id(16, 1_786_492_800), SpecId::CANCUN); // Coston
    }
}
