//! Transaction result types for the EVM chain.

use revm::context_interface::result::{ExecutionResult, Output};
use revm::inspector::Inspector;
use revm::interpreter::{CallInputs, CallOutcome, CreateInputs, CreateOutcome};
use revm::primitives::{Address, Bytes};

/// Result of a single EVM transaction execution.
#[derive(Debug, Clone)]
pub struct TransactionResult {
    pub success: bool,
    pub gas_used: u64,
    pub output: Option<Bytes>,
    pub logs: Vec<revm::primitives::Log>,
    pub created_address: Option<Address>,
}

impl From<ExecutionResult> for TransactionResult {
    fn from(result: ExecutionResult) -> Self {
        match result {
            ExecutionResult::Success {
                gas, logs, output, ..
            } => {
                let (out, addr) = match output {
                    Output::Call(b) => (Some(b), None),
                    Output::Create(b, addr) => (Some(b), addr),
                };
                Self {
                    success: true,
                    gas_used: gas.tx_gas_used(),
                    output: out,
                    logs,
                    created_address: addr,
                }
            }
            ExecutionResult::Revert {
                gas, logs, output, ..
            } => Self {
                success: false,
                gas_used: gas.tx_gas_used(),
                output: Some(output),
                logs,
                created_address: None,
            },
            ExecutionResult::Halt { gas, logs, .. } => Self {
                success: false,
                gas_used: gas.tx_gas_used(),
                output: None,
                logs,
                created_address: None,
            },
        }
    }
}

/// Raw call trace tree.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub roots: Vec<CallFrame>,
}

/// A single frame in a raw call trace.
#[derive(Debug, Clone)]
pub struct CallFrame {
    pub depth: usize,
    pub address: Option<Address>,
    pub input: Bytes,
    pub output: Bytes,
    pub gas_used: u64,
    pub success: bool,
    pub children: Vec<CallFrame>,
}

/// Raw trace inspector that collects [`CallFrame`] trees without formatting
/// or address labeling.
#[derive(Debug, Clone, Default)]
pub struct TraceInspector {
    stack: Vec<CallFrame>,
    roots: Vec<CallFrame>,
}

impl TraceInspector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn into_trace(self) -> Trace {
        Trace { roots: self.roots }
    }
}

impl<CTX: revm::context_interface::ContextTr> Inspector<CTX> for TraceInspector {
    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let input = inputs.input.bytes_local(_context.local());
        self.stack.push(CallFrame {
            depth: self.stack.len(),
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
