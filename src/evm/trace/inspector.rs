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

    fn load_fixture() -> Contract {
        let project = Project::new("fixtures/trace-inspector");
        let artifacts = project.load_artifacts().unwrap();
        let id =
            ArtifactId::try_from("src/BasicConstructorRevert.sol:BasicConstructorRevert").unwrap();
        Contract::try_get(&artifacts, &id).unwrap()
    }

    #[test]
    fn basic_constructor_revert_trace() {
        let contract = load_fixture();
        let mut chain = Chain::empty(Config::default().trace(true));
        let deployment = chain.deploy(DeployInput::new(&contract.initcode)).unwrap();
        assert!(
            !deployment.result.success,
            "deployment must fail because constructor reverts"
        );
        assert_eq!(deployment.trace.roots.len(), 1, "trace must have one root");

        let deploy_address = deployment.trace.roots[0].address.unwrap();
        let trace = deployment
            .trace
            .with_label(deploy_address, "BasicConstructorRevert");

        let formatted = format!("{trace}");
        let expected =
            fs::read_to_string("fixtures/trace-inspector/expected/BasicConstructorRevert.txt")
                .unwrap_or_else(|_| {
                    // If expected file doesn't exist, print the actual output for debugging
                    panic!("expected file not found. actual output:\n{formatted}")
                });
        assert_eq!(
            formatted.trim(),
            expected.trim(),
            "trace output must match expected"
        );
    }
}
