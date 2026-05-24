//! Chain-aware [`SpecId`] resolution.
//!
//! Uses `alloy-hardforks` and `alloy-op-hardforks` to derive the correct
//! revm [`SpecId`] for any supported chain at a given block timestamp.

use alloy_hardforks::EthereumHardfork;
use alloy_op_hardforks::OpHardfork;
use revm::primitives::hardfork::SpecId;

/// Map an [`EthereumHardfork`] into its corresponding [`SpecId`].
fn spec_id_from_ethereum_hardfork(hardfork: EthereumHardfork) -> SpecId {
    match hardfork {
        EthereumHardfork::Frontier => SpecId::FRONTIER,
        EthereumHardfork::Homestead => SpecId::HOMESTEAD,
        EthereumHardfork::Dao => SpecId::DAO_FORK,
        EthereumHardfork::Tangerine => SpecId::TANGERINE,
        EthereumHardfork::SpuriousDragon => SpecId::SPURIOUS_DRAGON,
        EthereumHardfork::Byzantium => SpecId::BYZANTIUM,
        EthereumHardfork::Constantinople => SpecId::CONSTANTINOPLE,
        EthereumHardfork::Petersburg => SpecId::PETERSBURG,
        EthereumHardfork::Istanbul => SpecId::ISTANBUL,
        EthereumHardfork::MuirGlacier => SpecId::MUIR_GLACIER,
        EthereumHardfork::Berlin => SpecId::BERLIN,
        EthereumHardfork::London => SpecId::LONDON,
        EthereumHardfork::ArrowGlacier => SpecId::ARROW_GLACIER,
        EthereumHardfork::GrayGlacier => SpecId::GRAY_GLACIER,
        EthereumHardfork::Paris => SpecId::MERGE,
        EthereumHardfork::Shanghai => SpecId::SHANGHAI,
        EthereumHardfork::Cancun => SpecId::CANCUN,
        EthereumHardfork::Prague => SpecId::PRAGUE,
        EthereumHardfork::Osaka => SpecId::OSAKA,
        EthereumHardfork::Amsterdam => SpecId::AMSTERDAM,
        EthereumHardfork::Bpo1
        | EthereumHardfork::Bpo2
        | EthereumHardfork::Bpo3
        | EthereumHardfork::Bpo4
        | EthereumHardfork::Bpo5 => SpecId::AMSTERDAM,
        _ => SpecId::AMSTERDAM,
    }
}

/// Map an [`OpHardfork`] into the closest corresponding [`SpecId`].
///
/// Optimism hardforks bundle Ethereum hardforks (e.g. Canyon brings
/// Shanghai, Ecotone brings Cancun).  This mapping selects the bundled
/// Ethereum spec so that opcode behaviour, gas costs, and validation
/// rules are correct for fuzzing.
fn spec_id_from_op_hardfork(hardfork: OpHardfork) -> SpecId {
    match hardfork {
        OpHardfork::Bedrock | OpHardfork::Regolith => SpecId::MERGE,
        OpHardfork::Canyon => SpecId::SHANGHAI,
        OpHardfork::Ecotone | OpHardfork::Fjord | OpHardfork::Granite | OpHardfork::Holocene => {
            SpecId::CANCUN
        }
        OpHardfork::Isthmus | OpHardfork::Jovian => SpecId::PRAGUE,
        OpHardfork::Karst | OpHardfork::Interop => SpecId::OSAKA,
        _ => SpecId::AMSTERDAM,
    }
}

/// Derive the correct [`SpecId`] for a forked block using chain-aware
/// hardfork tables from `alloy-hardforks` and `alloy-op-hardforks`.
///
/// Supported L1 chains: Ethereum mainnet, Sepolia, Holesky, Hoodi.
/// Supported L2 chains: Arbitrum, Arbitrum Sepolia, Optimism,
/// Optimism Sepolia, Base, Base Sepolia.
/// All other chains default to [`SpecId::AMSTERDAM`].
pub fn get_spec_id(chain_id: u64, timestamp: u64) -> SpecId {
    let chain = alloy_chains::Chain::from_id(chain_id);
    alloy_hardforks::EthereumHardfork::from_chain_and_timestamp(chain, timestamp)
        .map(spec_id_from_ethereum_hardfork)
        .or_else(|| {
            alloy_op_hardforks::OpHardfork::from_chain_and_timestamp(chain, timestamp)
                .map(spec_id_from_op_hardfork)
        })
        .unwrap_or(SpecId::AMSTERDAM)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_genesis_is_frontier() {
        assert_eq!(get_spec_id(1, 0), SpecId::FRONTIER);
    }

    #[test]
    fn mainnet_shanghai_at_activation() {
        // Shanghai activated at timestamp 1681338455 on mainnet
        assert_eq!(get_spec_id(1, 1_681_338_455), SpecId::SHANGHAI);
    }

    #[test]
    fn mainnet_cancun_at_activation() {
        // Cancun activated at timestamp 1710338135 on mainnet
        assert_eq!(get_spec_id(1, 1_710_338_135), SpecId::CANCUN);
    }

    #[test]
    fn mainnet_prague_at_activation() {
        // Prague activated at timestamp 1746612311 on mainnet
        assert_eq!(get_spec_id(1, 1_746_612_311), SpecId::PRAGUE);
    }

    #[test]
    fn sepolia_post_prague() {
        // Sepolia Prague at timestamp 1744905600
        assert_eq!(get_spec_id(11_155_111, 1_744_905_600), SpecId::PRAGUE);
    }

    #[test]
    fn arbitrum_one_shanghai_at_activation() {
        // Arbitrum One Shanghai at timestamp 1708804873
        assert_eq!(get_spec_id(42_161, 1_708_804_873), SpecId::SHANGHAI);
    }

    #[test]
    fn base_mainnet_ecotone_is_cancun() {
        // Base mainnet Ecotone at timestamp 1710374401
        assert_eq!(get_spec_id(8453, 1_710_374_401), SpecId::CANCUN);
    }

    #[test]
    fn optimism_mainnet_pre_canyon_is_merge() {
        // Optimism mainnet Regolith at timestamp 1679077200
        assert_eq!(get_spec_id(10, 1_679_077_200), SpecId::MERGE);
    }

    #[test]
    fn unknown_chain_defaults_to_amsterdam() {
        assert_eq!(get_spec_id(999_999, 0), SpecId::AMSTERDAM);
    }
}
