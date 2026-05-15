use revm::{
    inspector::Inspector,
    interpreter::{Interpreter, interpreter::EthInterpreter, interpreter_types::Jumps},
};

pub const MAP_SIZE: usize = 65_536;

/// Global coverage map for single-threaded / test usage.
pub static mut COVERAGE_MAP: [u8; MAP_SIZE] = [0u8; MAP_SIZE];

/// Inspector that writes PC-hit counts into a coverage map.
#[derive(Debug)]
pub struct CoverageInspector {
    map: &'static mut [u8],
}

impl CoverageInspector {
    /// Create an inspector backed by an arbitrary mutable slice.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for reads and writes for `len` bytes and remain
    /// valid for the lifetime of the inspector.
    pub unsafe fn new(ptr: *mut u8, len: usize) -> Self {
        Self {
            map: unsafe { std::slice::from_raw_parts_mut(ptr, len) },
        }
    }

    /// Create an inspector backed by the global `COVERAGE_MAP`.
    pub fn global() -> Self {
        unsafe { Self::new(std::ptr::addr_of_mut!(COVERAGE_MAP).cast::<u8>(), MAP_SIZE) }
    }
}

impl<CTX> Inspector<CTX, EthInterpreter> for CoverageInspector {
    fn step(&mut self, interp: &mut Interpreter<EthInterpreter>, _context: &mut CTX) {
        let pc = interp.bytecode.pc();
        if pc < self.map.len() {
            let idx = pc % self.map.len();
            self.map[idx] = self.map[idx].saturating_add(1);
        }
    }
}
