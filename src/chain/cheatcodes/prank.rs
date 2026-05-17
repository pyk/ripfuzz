//! Prank cheatcodes.

use revm::primitives::{Address, Bytes};

use crate::chain::cheatcodes::{
    Cheatcode, CheatcodeEffect, PrankState, StartPrankState, decode_address_arg,
};

pub struct Prank;
impl Cheatcode for Prank {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0xca, 0x66, 0x9f, 0xa7];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }
    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetPrank(PrankState {
            caller: addr,
            origin: addr,
            single_call: true,
            set_depth: 0,
            here: false,
        })]
    }
}

pub struct PrankHere;
impl Cheatcode for PrankHere {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x2b, 0x8d, 0xac, 0x2d];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }
    fn effects(_addr: Self::Args) -> Vec<CheatcodeEffect> {
        // prankHere needs the current depth, which is only available at frame_start.
        // We use a placeholder here and fix it up in apply_effect via inspector state.
        // Actually, the plan says apply_prank lives in frame_start and uses self.depth.
        // For the cheatcode dispatch, we return a generic prank that the inspector
        // will configure with the correct depth in apply_effect.
        //
        // Wait, apply_effect doesn't know the depth. Let's use set_depth = 0 as a
        // sentinel and have apply_prank in frame_start patch it.
        // Actually, the original code set set_depth = inspector.depth - 1 at call time.
        // With frame_start, the depth has already incremented. The cheatcode call itself
        // happens at depth N, so the parent frame depth is N-1. We need to capture this.
        //
        // Since the inspector runs the cheatcode in `call()`, self.depth is already
        // incremented by frame_start. So we can set set_depth = self.depth - 1 in
        // apply_effect if the prank has `here = true`.
        //
        // But apply_effect doesn't take `&self` (the inspector). It takes state.
        // Let's add the depth into PrankState in apply_effect by checking if here is true.
        // Actually no - PrankState already has a `here` field. In frame_start's apply_prank,
        // it checks if self.depth == prank.set_depth + 1. For prankHere, we need set_depth
        // to be the depth at which it was configured.
        //
        // Since the cheatcode is called inside `call()`, at that point `self.depth` has
        // been incremented by frame_start to the depth of the VM call. The parent frame
        // is `self.depth - 1`. So we can set `set_depth = self.depth - 1`.
        // But `effects()` doesn't have access to `self`.
        //
        // Solution: Return the prank with `here = true` and `set_depth = 0`. In the
        // inspector's `call()` method (where dispatch happens), patch the `set_depth`
        // for any prank effect that has `here = true`.
        vec![CheatcodeEffect::SetPrank(PrankState {
            caller: _addr,
            origin: _addr,
            single_call: true,
            set_depth: 0,
            here: true,
        })]
    }
}

pub struct StartPrank;
impl Cheatcode for StartPrank {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x45, 0xf5, 0x7d, 0x02];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }
    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetStartPrank(StartPrankState {
            caller: addr,
            origin: addr,
            set_depth: 0,
        })]
    }
}

pub struct StopPrank;
impl Cheatcode for StopPrank {
    type Args = ();
    const SELECTOR: [u8; 4] = [0xde, 0x00, 0x34, 0x7e];
    fn decode(_input: &Bytes) -> Option<Self::Args> {
        Some(())
    }
    fn effects(_: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::ClearPrank]
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use revm::primitives::Address;
    use serial_test::serial;

    use super::*;
    use crate::chain::Chain;
    use crate::contract;
    use crate::corpus::Call;

    #[test]
    fn prank_sets_single_call_state() {
        let addr = Address::new([0xab; 20]);
        let effects = Prank::effects(addr);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetPrank(PrankState {
                caller: addr,
                origin: addr,
                single_call: true,
                set_depth: 0,
                here: false,
            })]
        );
    }

    #[test]
    fn prank_here_sets_here_flag() {
        let addr = Address::new([0xcd; 20]);
        let effects = PrankHere::effects(addr);
        let CheatcodeEffect::SetPrank(p) = &effects[0] else {
            panic!("expected SetPrank");
        };
        assert!(p.here);
        assert_eq!(p.set_depth, 0);
    }

    #[test]
    fn start_prank_sets_persistent_state() {
        let addr = Address::new([0xef; 20]);
        let effects = StartPrank::effects(addr);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetStartPrank(StartPrankState {
                caller: addr,
                origin: addr,
                set_depth: 0,
            })]
        );
    }

    #[test]
    fn stop_prank_clears_all_prank_state() {
        let effects = StopPrank::effects(());
        assert_eq!(effects, vec![CheatcodeEffect::ClearPrank]);
    }

    #[test]
    #[serial]
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
