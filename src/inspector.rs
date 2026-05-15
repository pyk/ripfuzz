use revm::{
    inspector::Inspector,
    interpreter::{interpreter::EthInterpreter, interpreter_types::Jumps, Interpreter},
};

pub const MAP_SIZE: usize = 65_536;
pub static mut COVERAGE_MAP: [u8; MAP_SIZE] = [0u8; MAP_SIZE];

#[derive(Debug, Clone, Default)]
pub struct CoverageInspector;

impl<CTX> Inspector<CTX, EthInterpreter> for CoverageInspector {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let pc = interp.bytecode.pc();
        if pc < MAP_SIZE {
            unsafe {
                let idx = pc % MAP_SIZE;
                COVERAGE_MAP[idx] = COVERAGE_MAP[idx].saturating_add(1);
            }
        }
    }
}
