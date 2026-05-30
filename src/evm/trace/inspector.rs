//! Raw trace inspector that collects [`CallFrame`] trees without formatting
//! or address labeling.

use revm::inspector::Inspector as RevmInspector;
use revm::interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome};
use revm::primitives::Bytes;

use crate::evm::trace::CallFrame;
use crate::evm::trace::CallFrameKind;
use crate::evm::trace::Trace;

/// Raw trace inspector that collects [`CallFrame`] trees without formatting
/// or address labeling.
#[derive(Debug, Clone, Default)]
pub struct Inspector {
    stack: Vec<CallFrame>,
    roots: Vec<CallFrame>,
}

impl Inspector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_trace(self) -> Trace {
        Trace::new(self.roots)
    }
}

impl<CTX: revm::context_interface::ContextTr> RevmInspector<CTX> for Inspector {
    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(_context.local());
        self.stack.push(CallFrame {
            depth: self.stack.len(),
            kind: CallFrameKind::Call,
            address: Some(inputs.target_address),
            input,
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
        });
        None
    }

    fn call_end(&mut self, _context: &mut CTX, _inputs: &CallInputs, outcome: &mut CallOutcome) {
        let mut frame = self.stack.pop().unwrap_or_else(|| CallFrame {
            depth: 0,
            kind: CallFrameKind::Call,
            address: None,
            input: Bytes::new(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
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
            input: inputs.init_code().clone(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
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
            input: Bytes::new(),
            output: Bytes::new(),
            gas_used: 0,
            success: false,
            children: Vec::new(),
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
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::evm::Contract;
    use crate::evm::chain::{Chain, Config, DeployInput};
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
        ];

        for case in &cases {
            let contract = load_fixture(case.artifact_id);
            let mut chain = Chain::empty(Config::default().trace(true));
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
            let trace = deployment.trace.with_label(deploy_address, case.label);
            let trace = if case.with_abi {
                trace.with_abi(contract.abi)
            } else {
                trace
            };

            let formatted = format!("{trace}");
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
}
