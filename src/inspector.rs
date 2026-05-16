//! Coverage inspector that records EVM program counter hits.

use revm::{
    inspector::Inspector,
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
};

pub const MAP_SIZE: usize = 65_536;

/// Inspector that writes PC-hit counts into a coverage map.
#[derive(Debug)]
pub struct CoverageInspector<'a> {
    map: &'a mut [u8],
}

impl<'a> CoverageInspector<'a> {
    /// Create an inspector backed by a mutable byte slice.
    pub fn from_slice(map: &'a mut [u8]) -> Self {
        Self { map }
    }
}

impl<'a, CTX> Inspector<CTX, EthInterpreter> for CoverageInspector<'a> {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let pc = interp.bytecode.pc();
        if pc < self.map.len() {
            let idx = pc % self.map.len();
            self.map[idx] = self.map[idx].saturating_add(1);
        }
    }
}
