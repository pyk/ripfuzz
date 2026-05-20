//! Prank cheatcodes: vm.prank, vm.startPrank, vm.stopPrank.

use revm::primitives::{Address, Bytes};

use crate::vm::{Cheatcode, CheatcodeEffect, PrankState, StartPrankState, decode_address_arg};

// ---------------------------------------------------------------------------
// vm.prank(address)
// ---------------------------------------------------------------------------
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
            origin: None,
            single_call: true,
            set_depth: 0,
            prank_caller: Address::ZERO,
            used: false,
        })]
    }
}

// ---------------------------------------------------------------------------
// vm.prank(address,address)
// ---------------------------------------------------------------------------
pub struct PrankOrigin;
impl Cheatcode for PrankOrigin {
    type Args = (Address, Address);
    const SELECTOR: [u8; 4] = [0x47, 0xe5, 0x0c, 0xce];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 64 {
            return None;
        }
        let a = Address::from_slice(&input[4 + 12..4 + 32]);
        let b = Address::from_slice(&input[4 + 32 + 12..4 + 64]);
        Some((a, b))
    }
    fn effects((caller, origin): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetPrank(PrankState {
            caller,
            origin: Some(origin),
            single_call: true,
            set_depth: 0,
            prank_caller: Address::ZERO,
            used: false,
        })]
    }
}

// ---------------------------------------------------------------------------
// vm.startPrank(address)
// ---------------------------------------------------------------------------
pub struct StartPrank;
impl Cheatcode for StartPrank {
    type Args = Address;
    const SELECTOR: [u8; 4] = [0x06, 0x44, 0x7d, 0x56];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        decode_address_arg(input)
    }
    fn effects(addr: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetStartPrank(StartPrankState {
            caller: addr,
            origin: None,
            set_depth: 0,
            prank_caller: Address::ZERO,
            used: false,
        })]
    }
}

// ---------------------------------------------------------------------------
// vm.startPrank(address,address)
// ---------------------------------------------------------------------------
pub struct StartPrankOrigin;
impl Cheatcode for StartPrankOrigin {
    type Args = (Address, Address);
    const SELECTOR: [u8; 4] = [0x45, 0xb5, 0x60, 0x78];
    fn decode(input: &Bytes) -> Option<Self::Args> {
        if input.len() < 4 + 64 {
            return None;
        }
        let a = Address::from_slice(&input[4 + 12..4 + 32]);
        let b = Address::from_slice(&input[4 + 32 + 12..4 + 64]);
        Some((a, b))
    }
    fn effects((caller, origin): Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::SetStartPrank(StartPrankState {
            caller,
            origin: Some(origin),
            set_depth: 0,
            prank_caller: Address::ZERO,
            used: false,
        })]
    }
}

// ---------------------------------------------------------------------------
// vm.stopPrank()
// ---------------------------------------------------------------------------
pub struct StopPrank;
impl Cheatcode for StopPrank {
    type Args = ();
    const SELECTOR: [u8; 4] = [0x90, 0xc5, 0x01, 0x3b];
    fn decode(_input: &Bytes) -> Option<Self::Args> {
        Some(())
    }
    fn effects(_: Self::Args) -> Vec<CheatcodeEffect> {
        vec![CheatcodeEffect::ClearPrank]
    }
}

#[cfg(test)]
mod tests {

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
                origin: None,
                single_call: true,
                set_depth: 0,
                prank_caller: Address::ZERO,
                used: false,
            })]
        );
    }

    #[test]
    fn prank_origin_sets_both() {
        let caller = Address::new([0xab; 20]);
        let origin = Address::new([0xcd; 20]);
        let effects = PrankOrigin::effects((caller, origin));
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetPrank(PrankState {
                caller,
                origin: Some(origin),
                single_call: true,
                set_depth: 0,
                prank_caller: Address::ZERO,
                used: false,
            })]
        );
    }

    #[test]
    fn start_prank_sets_persistent_state() {
        let addr = Address::new([0xef; 20]);
        let effects = StartPrank::effects(addr);
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetStartPrank(StartPrankState {
                caller: addr,
                origin: None,
                set_depth: 0,
                prank_caller: Address::ZERO,
                used: false,
            })]
        );
    }

    #[test]
    fn start_prank_origin_sets_both() {
        let caller = Address::new([0xab; 20]);
        let origin = Address::new([0xcd; 20]);
        let effects = StartPrankOrigin::effects((caller, origin));
        assert_eq!(
            effects,
            vec![CheatcodeEffect::SetStartPrank(StartPrankState {
                caller,
                origin: Some(origin),
                set_depth: 0,
                prank_caller: Address::ZERO,
                used: false,
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
    fn prank_sender_only() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x8b, 0xa4, 0x07, 0x24]; // call_prank_sender()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_sender_and_origin() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x32, 0x69, 0xe0, 0x01]; // call_prank_origin()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_consumed_once() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0xcf, 0xc6, 0x97, 0xd1]; // call_prank_consumed()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn start_stop() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x9c, 0x86, 0xb2, 0x45]; // call_start_stop()
        let output = chain
            .execute_with_opts(
                &[Call {
                    selector: call,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                }],
                crate::chain::ExecutionOptions { trace: true },
            )
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn debug_start_stop() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_start: [u8; 4] = [0x9c, 0x86, 0xb2, 0x45]; // call_start_stop()
        // Trace is printed by the start_stop test above.
        chain
            .execute(&[Call {
                selector: call_start,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
    }

    #[test]
    #[serial]
    fn start_persists_across_sequence_calls() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_start: [u8; 4] = [0xa4, 0x03, 0xd2, 0x7d]; // call_start_no_stop()
        let call_after: [u8; 4] = [0x57, 0xee, 0x3f, 0x81]; // call_after_start_no_stop()
        let output = chain
            .execute(&[
                Call {
                    selector: call_start,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: call_after,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn setup_start_prank_persists() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodePrankSetup.sol",
        )
        .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x3e, 0xa0, 0x27, 0xaf]; // call_expect_persisted()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_does_not_persist_across_calls() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_prank: [u8; 4] = [0x8b, 0xa4, 0x07, 0x24]; // call_prank_sender()
        let call_consumed: [u8; 4] = [0xcf, 0xc6, 0x97, 0xd1]; // call_prank_consumed()
        let output = chain
            .execute(&[
                Call {
                    selector: call_prank,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: call_consumed,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_nested_not_leaked() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x3f, 0xf7, 0x0f, 0xda]; // call_prank_nested()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn start_prank_nested() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x6f, 0x61, 0x93, 0xa3]; // call_start_nested()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn stop_prank_mid_sequence() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_start: [u8; 4] = [0xa4, 0x03, 0xd2, 0x7d]; // call_start_no_stop()
        let call_stop: [u8; 4] = [0xa6, 0xb0, 0x8f, 0x5f]; // call_stop_mid()
        let call_after: [u8; 4] = [0x96, 0x9a, 0x98, 0xbd]; // call_after_stop()
        let output = chain
            .execute(&[
                Call {
                    selector: call_start,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: call_stop,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: call_after,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(output.all_ok);
        // After stopPrank, the third call should be unpranked.
    }

    #[test]
    #[serial]
    fn prank_constructor() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x46, 0x32, 0x20, 0x46]; // call_prank_constructor()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_modifier() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x50, 0xb1, 0x17, 0x4e]; // call_modifier_prank()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_modifier_cleanup() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_mod: [u8; 4] = [0x50, 0xb1, 0x17, 0x4e]; // call_modifier_prank()
        let call_after: [u8; 4] = [0x96, 0x9a, 0x98, 0xbd]; // call_after_stop()
        let output = chain
            .execute(&[
                Call {
                    selector: call_mod,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
                Call {
                    selector: call_after,
                    args: vec![],
                    block_number_delay: 0,
                    block_timestamp_delay: 0,
                    ..Default::default()
                },
            ])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn revert_discards_start_prank() {
        let artifact = contract::tests::load_test_artifact(
            "fixtures/cheatcodes",
            "test/CheatcodePrankSetup.sol",
        )
        .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call_revert: [u8; 4] = [0x90, 0x37, 0x8f, 0xba]; // call_override_and_revert()
        let output = chain
            .execute(&[Call {
                selector: call_revert,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!output.all_ok);

        // A fresh sequence must still see the setUp startPrank (0x888), not
        // the discarded 0x999.
        let call_expect: [u8; 4] = [0x3e, 0xa0, 0x27, 0xaf]; // call_expect_persisted()
        let output2 = chain
            .execute(&[Call {
                selector: call_expect,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output2.all_ok);
    }

    #[test]
    #[serial]
    fn start_prank_overwrite_used() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0xe7, 0xb4, 0x28, 0x84]; // call_start_overwrite_used()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(output.all_ok);
    }

    #[test]
    #[serial]
    fn start_prank_overwrite_unused_reverts() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x6a, 0x0f, 0xa3, 0x90]; // call_start_overwrite_unused_reverts()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!output.all_ok);
    }

    #[test]
    #[serial]
    fn prank_over_start_reverts() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x6e, 0x68, 0xaf, 0x8e]; // call_prank_over_start_reverts()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!output.all_ok);
    }

    #[test]
    #[serial]
    fn double_prank_reverts() {
        let artifact =
            contract::tests::load_test_artifact("fixtures/cheatcodes", "test/CheatcodePrank.sol")
                .unwrap();
        let chain = Chain::for_artifact(&artifact)
            .with_vm(crate::vm::Vm::new(crate::vm::VmConfig::default()))
            .init()
            .unwrap()
            .setup()
            .unwrap();
        let call: [u8; 4] = [0x4c, 0x7b, 0x41, 0x90]; // call_double_prank_reverts()
        let output = chain
            .execute(&[Call {
                selector: call,
                args: vec![],
                block_number_delay: 0,
                block_timestamp_delay: 0,
                ..Default::default()
            }])
            .unwrap();
        assert!(!output.all_ok);
    }
}
