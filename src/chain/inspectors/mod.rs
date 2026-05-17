//! Inspector composition for raptor's chain abstraction.

use std::sync::{Arc, RwLock};

use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::ContextTr,
    database::InMemoryDB,
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter,
        interpreter::EthInterpreter,
    },
};

use coverage::CoverageInspector;
use trace::TraceInspector;

use crate::chain::cheatcodes::CheatcodeInspector;

pub mod coverage;
pub mod trace;

/// Composite inspector that runs coverage, optional trace collection, and cheatcodes.
#[derive(Debug)]
pub struct CompositeInspector {
    pub coverage: CoverageInspector,
    pub trace: Option<TraceInspector>,
    pub cheatcodes: Option<CheatcodeInspector>,
}

impl CompositeInspector {
    /// Build a composite inspector with coverage and optional trace.
    pub fn new(coverage: CoverageInspector, trace: Option<TraceInspector>) -> Self {
        Self {
            coverage,
            trace,
            cheatcodes: None,
        }
    }

    /// Enable cheatcodes, optionally seeding from a persisted `CheatcodeState`,
    /// and wire shared label storage into the trace inspector.
    pub fn with_cheatcodes(mut self, state: crate::chain::cheatcodes::CheatcodeState) -> Self {
        let shared_labels = Arc::new(RwLock::new(state.labels.clone()));
        if let Some(ref mut t) = self.trace {
            t.set_shared_labels(Arc::clone(&shared_labels));
        }
        self.cheatcodes = Some(
            crate::chain::cheatcodes::CheatcodeInspector::from_state(state)
                .with_shared_labels(shared_labels),
        );
        self
    }
}

impl<CTX: ContextTr<Db = InMemoryDB, Block = BlockEnv, Tx = TxEnv> + ContextSetters>
    Inspector<CTX, EthInterpreter> for CompositeInspector
{
    fn initialize_interp(&mut self, interp: &mut Interpreter<EthInterpreter>, context: &mut CTX) {
        self.coverage.initialize_interp(interp, context);
        if let Some(ref mut t) = self.trace {
            t.initialize_interp(interp, context);
        }
        if let Some(ref mut c) = self.cheatcodes {
            c.initialize_interp(interp, context);
        }
    }

    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, context: &mut CTX) {
        self.coverage.step(interp, context);
        if let Some(ref mut t) = self.trace {
            t.step(interp, context);
        }
        if let Some(ref mut c) = self.cheatcodes {
            c.step(interp, context);
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter<EthInterpreter>, context: &mut CTX) {
        self.coverage.step_end(interp, context);
        if let Some(ref mut t) = self.trace {
            t.step_end(interp, context);
        }
        if let Some(ref mut c) = self.cheatcodes {
            c.step_end(interp, context);
        }
    }

    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        // Always call coverage and trace first to maintain their internal
        // stack state.  Cheatcodes may override the outcome afterwards.
        let cov = self.coverage.call(context, inputs);
        let tr = self.trace.as_mut().and_then(|t| t.call(context, inputs));

        if let Some(ref mut c) = self.cheatcodes
            && let Some(outcome) = c.call(context, inputs)
        {
            return Some(outcome);
        }

        tr.or(cov)
    }

    fn call_end(&mut self, context: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.coverage.call_end(context, inputs, outcome);
        if let Some(ref mut t) = self.trace {
            t.call_end(context, inputs, outcome);
        }
        if let Some(ref mut c) = self.cheatcodes {
            c.call_end(context, inputs, outcome);
        }
    }

    fn create(&mut self, context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        let cov = self.coverage.create(context, inputs);
        let tr = self.trace.as_mut().and_then(|t| t.create(context, inputs));
        tr.or(cov)
    }

    fn create_end(
        &mut self,
        context: &mut CTX,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.coverage.create_end(context, inputs, outcome);
        if let Some(ref mut t) = self.trace {
            t.create_end(context, inputs, outcome);
        }
    }
}
