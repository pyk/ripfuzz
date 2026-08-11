//! Raw trace inspector that collects [`CallFrame`] trees without formatting
//! or address labeling.

use std::collections::HashMap;

use alloy_primitives::{Address, B256, U256, keccak256};
use revm::bytecode::opcode::{KECCAK256, SSTORE};
use revm::context::JournalTr;
use revm::context_interface::Block;
use revm::inspector::Inspector as RevmInspector;
use revm::interpreter::interpreter_types::{InputsTr, Jumps};
use revm::interpreter::{CallInputs, CallOutcome, CallScheme, CreateInputs, CreateOutcome};
use revm::primitives::Bytes;

use crate::evm::trace::CallFrame;
use crate::evm::trace::CallFrameKind;
use crate::evm::trace::MappingSlots;
use crate::evm::trace::StorageChange;
use crate::evm::trace::Trace;

/// Raw trace inspector that collects [`CallFrame`] trees without formatting
/// or address labeling.
#[derive(Debug, Clone, Default)]
pub struct Inspector {
    stack: Vec<CallFrame>,
    roots: Vec<CallFrame>,
    mapping_slots: HashMap<Address, MappingSlots>,
}

impl Inspector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_trace(self) -> Trace {
        Trace {
            roots: self.roots,
            mapping_slots: self.mapping_slots,
        }
    }
}

impl<CTX: revm::context_interface::ContextTr> RevmInspector<CTX> for Inspector {
    fn step(&mut self, interp: &mut revm::interpreter::Interpreter, context: &mut CTX) {
        let address = interp.input.target_address();
        match interp.bytecode.opcode() {
            KECCAK256 => {
                let Ok(size) = interp.stack.peek(1) else {
                    return;
                };
                if size == U256::from(0x40) {
                    let Ok(offset) = interp.stack.peek(0) else {
                        return;
                    };
                    let data = interp.memory.slice_len(offset.saturating_to(), 0x40);
                    let key = B256::from_slice(&data[..0x20]);
                    let parent = B256::from_slice(&data[0x20..]);
                    let result = keccak256(&*data);
                    self.mapping_slots
                        .entry(address)
                        .or_default()
                        .record_sha3(result, key, parent);
                } else if size == U256::from(0x20) {
                    // Record 32-byte KECCAK256 as potential dynamic-array
                    // data area starts (e.g. keccak256(length_slot)).
                    let Ok(offset) = interp.stack.peek(0) else {
                        return;
                    };
                    let data = interp.memory.slice_len(offset.saturating_to(), 0x20);
                    let parent = B256::from_slice(&data[..0x20]);
                    let result = keccak256(&*data);
                    self.mapping_slots
                        .entry(address)
                        .or_default()
                        .record_array_start(result, parent);
                }
            }
            SSTORE => {
                let stack = interp.stack.data();
                let len = stack.len();
                if len < 2 {
                    return;
                }
                let slot = stack[len - 1];
                let new_value = stack[len - 2];
                let old_value = context
                    .journal_mut()
                    .sload_skip_cold_load(address, slot, true)
                    .ok()
                    .map(|s| s.data)
                    .unwrap_or_default();

                if let Some(slots) = self.mapping_slots.get_mut(&address) {
                    slots.insert_nearby(slot.into());
                }

                if let Some(frame) = self.stack.last_mut() {
                    frame.storage_changes.push(StorageChange {
                        slot,
                        old_value,
                        new_value,
                    });
                }
            }
            _ => {}
        }
    }

    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(_context.local());
        let (_hash, bytecode) = &inputs.known_bytecode;
        let code_bytes: Option<Bytes> = if bytecode.is_empty() {
            None
        } else {
            let raw = bytecode.original_bytes();
            if raw.is_empty() {
                None
            } else {
                Some(Bytes::from(raw.to_vec()))
            }
        };
        self.stack.push(CallFrame {
            depth: self.stack.len(),
            kind: CallFrameKind::Call(inputs.scheme),
            address: Some(inputs.target_address),
            code_address: Some(inputs.bytecode_address),
            code_bytes,
            caller: inputs.caller,
            value: inputs.value.get(),
            timestamp: _context.block().timestamp(),
            number: _context.block().number(),
            input,
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
            storage_changes: Vec::new(),
            logs: Vec::new(),
        });
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        let mut frame = self.stack.pop().unwrap_or_else(|| CallFrame {
            depth: 0,
            kind: CallFrameKind::Call(CallScheme::Call),
            address: None,
            code_address: None,
            code_bytes: None,
            caller: Address::ZERO,
            value: U256::ZERO,
            timestamp: U256::ZERO,
            number: U256::ZERO,
            input: Bytes::new(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
            storage_changes: Vec::new(),
            logs: Vec::new(),
        });
        let ir = &outcome.result;
        frame.gas_used = ir.gas.total_gas_spent();
        frame.success = ir.result.is_ok();
        frame.output = ir.output.clone();

        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(frame);
        } else {
            self.roots.push(frame);
        }
    }

    fn create(&mut self, _context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.stack.push(CallFrame {
            depth: self.stack.len(),
            kind: CallFrameKind::Create,
            address: None,
            code_address: None,
            code_bytes: None,
            caller: inputs.caller(),
            value: inputs.value(),
            timestamp: _context.block().timestamp(),
            number: _context.block().number(),
            input: inputs.init_code().clone(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
            storage_changes: Vec::new(),
            logs: Vec::new(),
        });
        None
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        _inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        let mut frame = self.stack.pop().unwrap_or_else(|| CallFrame {
            depth: 0,
            kind: CallFrameKind::Create,
            address: None,
            code_address: None,
            code_bytes: None,
            caller: Address::ZERO,
            value: U256::ZERO,
            timestamp: U256::ZERO,
            number: U256::ZERO,
            input: Bytes::new(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
            storage_changes: Vec::new(),
            logs: Vec::new(),
        });
        let ir = &outcome.result;
        frame.gas_used = ir.gas.total_gas_spent();
        frame.success = ir.result.is_ok();
        frame.output = ir.output.clone();
        if let Some(addr) = outcome.address {
            frame.address = Some(addr);
        }

        if let Some(parent) = self.stack.last_mut() {
            parent.children.push(frame);
        } else {
            self.roots.push(frame);
        }
    }

    fn log(&mut self, _context: &mut CTX, log: revm::primitives::Log) {
        if let Some(frame) = self.stack.last_mut() {
            frame.logs.push(log);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use alloy_primitives::U256;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, ChainConfig, DeployInput};
    use crate::evm::trace::TraceContext;
    use crate::foundry::{ArtifactId, Project};

    struct TestCase {
        artifact_id: &'static str,
        label: &'static str,
        expected_file: &'static str,
        with_abi: bool,
    }

    fn load_fixture(artifact_id: &str) -> Contract {
        let project = Project::new("fixtures/trace-inspector");
        let artifacts = project.load_artifacts().unwrap();
        let id = ArtifactId::try_from(artifact_id).unwrap();
        Contract::try_get(&artifacts, &id).unwrap()
    }

    #[test]
    fn constructor_revert_traces() {
        let cases = [
            TestCase {
                artifact_id: "src/BasicConstructorRevert.sol:BasicConstructorRevert",
                label: "BasicConstructorRevert",
                expected_file: "fixtures/trace-inspector/expected/BasicConstructorRevert.txt",
                with_abi: false,
            },
            TestCase {
                artifact_id: "src/BasicConstructorCustomErrorRevert.sol:BasicConstructorCustomErrorRevert",
                label: "BasicConstructorCustomErrorRevert",
                expected_file: "fixtures/trace-inspector/expected/BasicConstructorCustomErrorRevert.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/BasicConstructorAssertionFailed.sol:BasicConstructorAssertionFailed",
                label: "BasicConstructorAssertionFailed",
                expected_file: "fixtures/trace-inspector/expected/BasicConstructorAssertionFailed.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicArithmeticOverflow.sol:PanicArithmeticOverflow",
                label: "PanicArithmeticOverflow",
                expected_file: "fixtures/trace-inspector/expected/PanicArithmeticOverflow.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicDivisionByZero.sol:PanicDivisionByZero",
                label: "PanicDivisionByZero",
                expected_file: "fixtures/trace-inspector/expected/PanicDivisionByZero.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicArrayOutOfBounds.sol:PanicArrayOutOfBounds",
                label: "PanicArrayOutOfBounds",
                expected_file: "fixtures/trace-inspector/expected/PanicArrayOutOfBounds.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicEnumConversionError.sol:PanicEnumConversionError",
                label: "PanicEnumConversionError",
                expected_file: "fixtures/trace-inspector/expected/PanicEnumConversionError.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicEmptyArrayPop.sol:PanicEmptyArrayPop",
                label: "PanicEmptyArrayPop",
                expected_file: "fixtures/trace-inspector/expected/PanicEmptyArrayPop.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicInvalidInternalFunction.sol:PanicInvalidInternalFunction",
                label: "PanicInvalidInternalFunction",
                expected_file: "fixtures/trace-inspector/expected/PanicInvalidInternalFunction.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/PanicResourceError.sol:PanicResourceError",
                label: "PanicResourceError",
                expected_file: "fixtures/trace-inspector/expected/PanicResourceError.txt",
                with_abi: true,
            },
            TestCase {
                artifact_id: "src/CustomErrorWithArgsRevert.sol:CustomErrorWithArgsRevert",
                label: "CustomErrorWithArgsRevert",
                expected_file: "fixtures/trace-inspector/expected/CustomErrorWithArgsRevert.txt",
                with_abi: true,
            },
        ];

        for case in &cases {
            let contract = load_fixture(case.artifact_id);
            let mut chain = Chain::empty(ChainConfig::default().trace(true));
            let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
            assert!(
                !deployment.result.success,
                "{}: deployment must fail",
                case.label
            );
            assert_eq!(
                deployment.trace.roots.len(),
                1,
                "{}: trace must have one root",
                case.label
            );

            let deploy_address = deployment.trace.roots[0].address.unwrap();
            let mut ctx = TraceContext::new().with_label(deploy_address, case.label);
            if case.with_abi {
                ctx = ctx.with_abi(contract.abi);
            }

            let formatted = format!("{}", deployment.trace.display_with(&ctx));
            let expected = fs::read_to_string(case.expected_file).unwrap_or_else(|_| {
                panic!(
                    "{}: expected file not found. actual output:\n{formatted}",
                    case.label
                )
            });
            assert_eq!(
                formatted.trim(),
                expected.trim(),
                "{}: trace output must match expected",
                case.label
            );
        }
    }

    #[test]
    fn constructor_complex_revert_trace() {
        let outer =
            load_fixture("src/BasicConstructorComplexRevert.sol:BasicConstructorComplexRevert");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let mut deploy_opts =
            DeployInput::new(&outer.initcode).value(alloy_primitives::U256::from(10000));
        for lib in &outer.libraries {
            deploy_opts = deploy_opts.add_library(lib.clone());
        }
        let deployment = chain.deploy(deploy_opts).unwrap();
        assert!(!deployment.result.success, "deployment must fail");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, outer.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected = fs::read_to_string(
            "fixtures/trace-inspector/expected/BasicConstructorComplexRevert.txt",
        )
        .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}",));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    #[test]
    fn storage_types_trace() {
        let outer = load_fixture("src/StorageTypes.sol:StorageTypesRevert");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&outer.initcode)).unwrap();
        assert!(!deployment.result.success, "deployment must fail");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, outer.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected =
            fs::read_to_string("fixtures/trace-inspector/expected/StorageTypesRevert.txt")
                .unwrap_or_else(
                    |_| panic!("expected file not found. actual output:\n{formatted}",),
                );
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    #[test]
    fn label_trace() {
        let contract = load_fixture("src/LabelTrace.sol:LabelTrace");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(!deployment.result.success, "deployment must fail");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());
        for (addr, label) in chain.labels() {
            ctx = ctx.with_label(*addr, label.clone());
        }

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected = fs::read_to_string("fixtures/trace-inspector/expected/LabelTrace.txt")
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}",));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    #[test]
    fn emit_events_trace() {
        let contract = load_fixture("src/EmitEvents.sol:EmitEvents");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected = fs::read_to_string("fixtures/trace-inspector/expected/EmitEvents.txt")
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}",));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    #[test]
    fn log_events_trace() {
        let contract = load_fixture("src/LogEvents.sol:LogEvents");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(deployment.result.success, "deployment must succeed");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected = fs::read_to_string("fixtures/trace-inspector/expected/LogEvents.txt")
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}",));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    /// Regression test: storage changes recorded before a revert must not be
    /// discarded when the call or create frame fails.
    #[test]
    fn storage_changes_not_cleared_on_revert() {
        let contract = load_fixture("src/StorageChangeRevert.sol:StorageChangeRevert");

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            !deployment.result.success,
            "deployment must fail when constructor reverts"
        );
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        assert!(
            !root.storage_changes.is_empty(),
            "storage changes must be preserved on revert"
        );
        assert_eq!(
            root.storage_changes.len(),
            1,
            "exactly one storage change must be recorded"
        );
        let change = &root.storage_changes[0];
        assert_eq!(change.old_value, U256::ZERO, "old value must be 0");
        assert_eq!(change.new_value, U256::from(42), "new value must be 42");
    }

    /// Regression test: storage changes inside a struct-valued mapping must be
    /// decoded even when the exact mapping base slot never appears in an SSTORE
    /// because the first field is never touched.
    #[test]
    fn struct_mapping_slot_decoded() {
        let contract = load_fixture("src/StructMappingSlot.sol:StructMappingSlot");

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            !deployment.result.success,
            "deployment must fail when constructor reverts"
        );
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        assert!(
            !root.storage_changes.is_empty(),
            "storage changes must be recorded"
        );
        assert_eq!(
            root.storage_changes.len(),
            1,
            "exactly one storage change must be recorded"
        );
        let change = &root.storage_changes[0];
        assert_eq!(change.new_value, U256::from(42), "new value must be 42");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();
        let deploy_address = root.address.unwrap();
        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        assert!(
            formatted.contains("data[1].c"),
            "mapping slot must be decoded as 'data[1].c', got:\n{formatted}"
        );
    }

    /// Regression test: storage changes inside a fixed array that is a field of
    /// a struct inside a mapping must be decoded even when the accessed slot is
    /// beyond the struct field's base slot.
    #[test]
    fn array_in_struct_mapping_slot_decoded() {
        let contract = load_fixture("src/ArrayInStructMapping.sol:ArrayInStructMapping");

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            !deployment.result.success,
            "deployment must fail when constructor reverts"
        );
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        assert!(
            !root.storage_changes.is_empty(),
            "storage changes must be recorded"
        );
        assert_eq!(
            root.storage_changes.len(),
            1,
            "exactly one storage change must be recorded"
        );
        let change = &root.storage_changes[0];
        assert_eq!(change.new_value, U256::from(42), "new value must be 42");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();
        let deploy_address = root.address.unwrap();
        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        assert!(
            formatted.contains("data[1].arr[1]"),
            "mapping slot must be decoded as 'data[1].arr[1]', got:\n{formatted}"
        );
    }

    /// Regression test: storage changes to a dynamic array of structs that
    /// contain nested struct fields must be decoded to human-readable labels
    /// (e.g. `entries[0].data.b`) instead of raw keccak hashes.
    #[test]
    fn array_of_nested_struct_trace() {
        let contract = load_fixture("src/ArrayOfNestedStruct.sol:ArrayOfNestedStruct");

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            !deployment.result.success,
            "deployment must fail when constructor reverts"
        );
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();
        let deploy_address = root.address.unwrap();
        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected =
            fs::read_to_string("fixtures/trace-inspector/expected/ArrayOfNestedStruct.txt")
                .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    #[test]
    fn return_value_types_trace() {
        let outer = load_fixture("src/ReturnValueTypesTrace.sol:ReturnValueTypesTrace");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&outer.initcode)).unwrap();
        assert!(!deployment.result.success, "deployment must fail");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();

        ctx = ctx.with_label(deploy_address, outer.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected =
            fs::read_to_string("fixtures/trace-inspector/expected/ReturnValueTypesTrace.txt")
                .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }

    /// Calls to addresses with no bytecode must annotate the empty account path
    /// so --stop-on-revert traces are actionable (e.g. forgot rvm.fork).
    #[test]
    fn call_empty_account_trace() {
        let contract = load_fixture("src/CallEmptyAccount.sol:CallEmptyAccount");

        let project = Project::new("fixtures/trace-inspector");
        let mut ctx = TraceContext::from_project(&project).unwrap();

        let mut chain = Chain::empty(ChainConfig::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(!deployment.result.success, "deployment must fail");
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let root = &deployment.trace.roots[0];
        let deploy_address = root.address.unwrap();
        ctx = ctx.with_label(deploy_address, contract.artifact_id.name.clone());

        let formatted = format!("{}", deployment.trace.display_with(&ctx));
        let expected = fs::read_to_string("fixtures/trace-inspector/expected/CallEmptyAccount.txt")
            .unwrap_or_else(|_| panic!("expected file not found. actual output:\n{formatted}"));
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }
}
