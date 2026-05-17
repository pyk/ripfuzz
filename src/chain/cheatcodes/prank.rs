//! Prank cheatcodes.

use revm::interpreter::CallOutcome;

use crate::chain::cheatcodes::{CheatcodeInspector, dummy_success};

/// `prank(address)` — single-call prank.
pub const PRANK_SELECTOR: [u8; 4] = [0xca, 0x66, 0x9f, 0xa7];
/// `prankHere(address)` — single-call prank from current origin.
pub const PRANK_HERE_SELECTOR: [u8; 4] = [0x2b, 0x8d, 0xac, 0x2d];
/// `startPrank(address)` — persistent prank.
pub const START_PRANK_SELECTOR: [u8; 4] = [0x45, 0xf5, 0x7d, 0x02];
/// `stopPrank()` — stop persistent prank.
pub const STOP_PRANK_SELECTOR: [u8; 4] = [0xde, 0x00, 0x34, 0x7e];

pub fn handle_prank(
    inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = super::decode_address_arg(input)?;
    inspector.state.prank = Some(super::PrankState {
        caller: addr,
        origin: addr,
        single_call: true,
        set_depth: 0,
        here: false,
    });
    Some(dummy_success())
}

pub fn handle_prank_here(
    inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = super::decode_address_arg(input)?;
    // prankHere changes msg.sender for the very next direct child call from
    // the current frame.  We record the parent frame's depth so the
    // inspector can target only that child call.
    inspector.state.prank = Some(super::PrankState {
        caller: addr,
        origin: addr,
        single_call: true,
        set_depth: inspector.depth.saturating_sub(1),
        here: true,
    });
    Some(dummy_success())
}

pub fn handle_start_prank(
    inspector: &mut CheatcodeInspector,
    input: &revm::primitives::Bytes,
) -> Option<CallOutcome> {
    let addr = super::decode_address_arg(input)?;
    inspector.state.start_prank = Some(super::StartPrankState {
        caller: addr,
        origin: addr,
        set_depth: inspector.depth.saturating_sub(1),
    });
    Some(dummy_success())
}

pub fn handle_stop_prank(inspector: &mut CheatcodeInspector) -> Option<CallOutcome> {
    inspector.state.prank = None;
    inspector.state.start_prank = None;
    Some(dummy_success())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::Address;

    use super::*;
    use crate::chain::Chain;
    use crate::chain::cheatcodes::CheatcodeInspector;
    use crate::contract;
    use crate::corpus::Call;

    fn call_data(selector: [u8; 4], addr: Address) -> revm::primitives::Bytes {
        let mut data = vec![0u8; 4 + 32];
        data[0..4].copy_from_slice(&selector);
        data[4 + 12..4 + 32].copy_from_slice(addr.as_slice());
        revm::primitives::Bytes::from(data)
    }

    #[test]
    fn prank_sets_single_call_state() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xab; 20]);
        let result = handle_prank(&mut inspector, &call_data(PRANK_SELECTOR, addr));
        assert!(result.is_some());
        let prank = inspector.state.prank.unwrap();
        assert_eq!(prank.caller, addr);
        assert_eq!(prank.origin, addr);
        assert!(prank.single_call);
        assert!(!prank.here);
    }

    #[test]
    fn prank_here_records_parent_depth() {
        let mut inspector = CheatcodeInspector::new();
        inspector.depth = 3;
        let addr = Address::new([0xcd; 20]);
        let result = handle_prank_here(&mut inspector, &call_data(PRANK_HERE_SELECTOR, addr));
        assert!(result.is_some());
        let prank = inspector.state.prank.unwrap();
        assert!(prank.here);
        assert_eq!(prank.set_depth, 2); // depth - 1
    }

    #[test]
    fn start_prank_sets_persistent_state() {
        let mut inspector = CheatcodeInspector::new();
        let addr = Address::new([0xef; 20]);
        let result = handle_start_prank(&mut inspector, &call_data(START_PRANK_SELECTOR, addr));
        assert!(result.is_some());
        let start = inspector.state.start_prank.unwrap();
        assert_eq!(start.caller, addr);
        assert_eq!(start.set_depth, 0);
    }

    #[test]
    fn stop_prank_clears_all_prank_state() {
        let mut inspector = CheatcodeInspector::new();
        inspector.state.prank = Some(super::super::PrankState {
            caller: Address::ZERO,
            origin: Address::ZERO,
            single_call: true,
            set_depth: 0,
            here: false,
        });
        inspector.state.start_prank = Some(super::super::StartPrankState {
            caller: Address::ZERO,
            origin: Address::ZERO,
            set_depth: 0,
        });
        let result = handle_stop_prank(&mut inspector);
        assert!(result.is_some());
        assert!(inspector.state.prank.is_none());
        assert!(inspector.state.start_prank.is_none());
    }

    #[test]
    fn cheatcode_prank_integration() {
        let artifact = contract::ContractBuilder::build(
            Path::new("fixtures/cheatcodes"),
            Path::new("test/CheatcodePrank.sol"),
        )
        .unwrap();

        let chain = Chain::initialize(&artifact).unwrap().setup().unwrap();
        let action_prank: [u8; 4] = [0xe8, 0x29, 0x58, 0x0d]; // action_prank()
        let action_start: [u8; 4] = [0xb1, 0x61, 0x16, 0x84]; // action_start_prank()

        let output = chain
            .execute(&vec![
                Call {
                    selector: action_prank,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: action_start,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(output.all_ok, "prank actions should succeed");
        assert!(
            output.property_results.iter().all(|p| p.passed),
            "prank property should pass"
        );
    }
}
