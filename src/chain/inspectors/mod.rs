//! Inspector composition for raptor's chain abstraction.

use revm::{
    inspector::Inspector,
    interpreter::{
        CallInputs, CallOutcome, CreateInputs, CreateOutcome, Interpreter, InterpreterTypes,
    },
    primitives::{Address, Log, U256},
};

pub mod trace;

/// Local wrapper around an optional [`TraceInspector`] so we can implement
/// the foreign `Inspector` trait.
#[derive(Debug)]
pub struct MaybeTrace(pub Option<trace::TraceInspector>);

impl<CTX, INTR, FI, FR> Inspector<CTX, INTR, FI, FR> for MaybeTrace
where
    INTR: InterpreterTypes,
    trace::TraceInspector: Inspector<CTX, INTR, FI, FR>,
{
    #[inline]
    fn initialize_interp(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        if let Some(ref mut t) = self.0 {
            t.initialize_interp(interp, context);
        }
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        if let Some(ref mut t) = self.0 {
            t.step(interp, context);
        }
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        if let Some(ref mut t) = self.0 {
            t.step_end(interp, context);
        }
    }

    #[inline]
    fn log(&mut self, context: &mut CTX, log: Log) {
        if let Some(ref mut t) = self.0 {
            t.log(context, log);
        }
    }

    #[inline]
    fn log_full(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX, log: Log) {
        if let Some(ref mut t) = self.0 {
            t.log_full(interp, context, log);
        }
    }

    #[inline]
    fn frame_start(&mut self, context: &mut CTX, frame_input: &mut FI) -> Option<FR> {
        self.0
            .as_mut()
            .and_then(|t| t.frame_start(context, frame_input))
    }

    #[inline]
    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        self.0.as_mut().and_then(|t| t.call(context, inputs))
    }

    #[inline]
    fn call_end(&mut self, context: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        if let Some(ref mut t) = self.0 {
            t.call_end(context, inputs, outcome);
        }
    }

    #[inline]
    fn create(&mut self, context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        self.0.as_mut().and_then(|t| t.create(context, inputs))
    }

    #[inline]
    fn create_end(
        &mut self,
        context: &mut CTX,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if let Some(ref mut t) = self.0 {
            t.create_end(context, inputs, outcome);
        }
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        if let Some(ref mut t) = self.0 {
            t.selfdestruct(contract, target, value);
        }
    }
}

/// Local 3-tuple wrapper so we can implement the foreign `Inspector` trait.
#[derive(Debug)]
pub struct InspectorTuple<A, B, C>(pub A, pub B, pub C);

impl<A, B, C> InspectorTuple<A, B, C> {
    pub fn new(a: A, b: B, c: C) -> Self {
        Self(a, b, c)
    }
}

impl<CTX, INTR, FI, FR, A, B, C> Inspector<CTX, INTR, FI, FR> for InspectorTuple<A, B, C>
where
    INTR: InterpreterTypes,
    A: Inspector<CTX, INTR, FI, FR>,
    B: Inspector<CTX, INTR, FI, FR>,
    C: Inspector<CTX, INTR, FI, FR>,
{
    #[inline]
    fn initialize_interp(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        self.0.initialize_interp(interp, context);
        self.1.initialize_interp(interp, context);
        self.2.initialize_interp(interp, context);
    }

    #[inline]
    fn step(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        self.0.step(interp, context);
        self.1.step(interp, context);
        self.2.step(interp, context);
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX) {
        self.0.step_end(interp, context);
        self.1.step_end(interp, context);
        self.2.step_end(interp, context);
    }

    #[inline]
    fn log(&mut self, context: &mut CTX, log: Log) {
        self.0.log(context, log.clone());
        self.1.log(context, log.clone());
        self.2.log(context, log);
    }

    #[inline]
    fn log_full(&mut self, interp: &mut Interpreter<INTR>, context: &mut CTX, log: Log) {
        self.0.log_full(interp, context, log.clone());
        self.1.log_full(interp, context, log.clone());
        self.2.log_full(interp, context, log);
    }

    #[inline]
    fn frame_start(&mut self, context: &mut CTX, frame_input: &mut FI) -> Option<FR> {
        let a = self.0.frame_start(context, frame_input);
        let b = self.1.frame_start(context, frame_input);
        let c = self.2.frame_start(context, frame_input);
        c.or(b).or(a)
    }

    #[inline]
    fn call(&mut self, context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let a = self.0.call(context, inputs);
        let b = self.1.call(context, inputs);
        let c = self.2.call(context, inputs);
        c.or(b).or(a)
    }

    #[inline]
    fn call_end(&mut self, context: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        self.0.call_end(context, inputs, outcome);
        self.1.call_end(context, inputs, outcome);
        self.2.call_end(context, inputs, outcome);
    }

    #[inline]
    fn create(&mut self, context: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        let a = self.0.create(context, inputs);
        let b = self.1.create(context, inputs);
        let c = self.2.create(context, inputs);
        c.or(b).or(a)
    }

    #[inline]
    fn create_end(
        &mut self,
        context: &mut CTX,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        self.0.create_end(context, inputs, outcome);
        self.1.create_end(context, inputs, outcome);
        self.2.create_end(context, inputs, outcome);
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        self.0.selfdestruct(contract, target, value);
        self.1.selfdestruct(contract, target, value);
        self.2.selfdestruct(contract, target, value);
    }
}
