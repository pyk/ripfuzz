//! Random value generation helpers seeded with extracted literals.

use fastrand::Rng;

pub use int::int;
pub use uint::uint;
pub mod int;
pub mod uint;

/// Pick a random item from a slice, or return `None` if empty.
pub fn pick_random<T: Clone>(items: &[T], rng: &mut Rng) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[rng.usize(0..items.len())].clone())
    }
}
