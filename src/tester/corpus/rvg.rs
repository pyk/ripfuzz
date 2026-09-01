//! Random argument value generation seeded with the extracted literals.
//!
//! [`RandomValueGenerator`] turns an ABI type into a value for a fuzzed call.
//! Each kind mixes three sources so the fuzzer both explores wide ranges and
//! lands on the exact constants the harness gates its assertions behind:
//!
//! - 20% chance to pick a literal from the extracted pool
//! - 30% chance to generate an edge case (`0`, `1`, `max`, `max - 1`, ...)
//! - 50% chance to generate a uniformly random value

use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, Bytes, FixedBytes, I256, U256};
use fastrand::Rng;

use crate::tester::corpus::literal::LiteralExtractor;

/// Generates random [`DynSolValue`]s for ABI types from an RNG and the
/// extracted literal pool.
pub struct RandomValueGenerator<'a> {
    rng: &'a mut Rng,
    literals: &'a LiteralExtractor,
}

impl<'a> RandomValueGenerator<'a> {
    pub fn new(rng: &'a mut Rng, literals: &'a LiteralExtractor) -> Self {
        Self { rng, literals }
    }

    /// Generate a random value for the given ABI type.
    pub fn value(&mut self, ty: &DynSolType) -> DynSolValue {
        match ty {
            DynSolType::Bool => DynSolValue::Bool(self.bool()),
            DynSolType::Uint(sz) => DynSolValue::Uint(self.uint(*sz), *sz),
            DynSolType::Int(sz) => DynSolValue::Int(self.int(*sz), *sz),
            DynSolType::FixedBytes(sz) => DynSolValue::FixedBytes(self.fixed_bytes(*sz), *sz),
            DynSolType::Address => DynSolValue::Address(self.address()),
            DynSolType::Bytes => DynSolValue::Bytes(self.bytes().to_vec()),
            DynSolType::String => DynSolValue::String(self.string()),
            DynSolType::Function => {
                let mut bytes = [0u8; 24];
                self.rng.fill(&mut bytes);
                DynSolValue::Function(alloy_primitives::Function::from_slice(&bytes))
            }
            DynSolType::Array(inner) => {
                let len = self.rng.usize(0..=4);
                let arr: Vec<DynSolValue> = (0..len).map(|_| self.value(inner)).collect();
                DynSolValue::Array(arr)
            }
            DynSolType::FixedArray(inner, len) => {
                let arr: Vec<DynSolValue> = (0..*len).map(|_| self.value(inner)).collect();
                DynSolValue::FixedArray(arr)
            }
            DynSolType::Tuple(types) => {
                let values: Vec<DynSolValue> = types.iter().map(|ty| self.value(ty)).collect();
                DynSolValue::Tuple(values)
            }
        }
    }

    /// Generate a random boolean value.
    ///
    /// Returns `true` 50% of the time and `false` 50% of the time.
    fn bool(&mut self) -> bool {
        self.rng.bool()
    }

    /// Generate a random unsigned integer of the given bit width.
    fn uint(&mut self, bits: usize) -> U256 {
        let max = max_for_bits(bits);
        let group = self.literals.uint(bits).to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group)
                && val <= max
            {
                return val;
            }
        } else if roll < 50 {
            let edge = match self.rng.u32(0..6) {
                0 => U256::ZERO,
                1 => U256::from(1),
                2 => max,
                3 => max.saturating_sub(U256::from(1)),
                4 => max.saturating_sub(U256::from(2)),
                _ => max.saturating_sub(U256::from(3)),
            };
            return edge;
        }

        let mut bytes = [0u8; 32];
        self.rng.fill(&mut bytes);
        let raw = U256::from_be_bytes::<32>(bytes);
        if bits == 256 { raw } else { raw & max }
    }

    /// Generate a random signed integer of the given bit width.
    fn int(&mut self, bits: usize) -> I256 {
        // The minimum is the sign bit sign-extended to 256 bits: a raw
        // `from_raw(sign_bit(bits))` is positive for every width below 256
        // and would make the literal range check below impossible.
        let min = sign_extend(sign_bit(bits), bits);
        let max_positive = I256::from_raw(max_positive_for_bits(bits));
        let group = self.literals.int(bits).to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group)
                && val >= min
                && val <= max_positive
            {
                return val;
            }
        } else if roll < 50 {
            let raw = match self.rng.u32(0..7) {
                0 => sign_bit(bits),                              // min
                1 => sign_bit(bits) + U256::from(1),              // min + 1
                2 => mask(bits),                                  // -1
                3 => U256::ZERO,                                  // 0
                4 => U256::from(1),                               // 1
                5 => max_positive_for_bits(bits) - U256::from(1), // max - 1
                _ => max_positive_for_bits(bits),                 // max
            };
            return sign_extend(raw, bits);
        }

        let mut bytes = [0u8; 32];
        self.rng.fill(&mut bytes);
        let raw = U256::from_be_bytes::<32>(bytes);
        sign_extend(raw, bits)
    }

    /// Generate a random fixed-size byte sequence.
    ///
    /// `size` is the byte size (e.g. 4 for `bytes4`, 32 for `bytes32`).
    fn fixed_bytes(&mut self, size: usize) -> FixedBytes<32> {
        let group = self.literals.fixed_bytes(size).to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group) {
                // Left-align the value into the word, because the encoding
                // of `bytesN` takes its first `size` bytes and Solidity
                // compares the value in those bytes.
                let word = (val << (256 - 8 * size as u32)).to_be_bytes::<32>();
                return FixedBytes::from(word);
            }
        } else if roll < 50 {
            let mut word = [0u8; 32];
            match self.rng.u32(0..6) {
                0 => {}                       // all zeros
                1 => word[0] = 1,             // 1 in first byte
                2 => word[..size].fill(0xFF), // max
                3 => {
                    word[..size].fill(0xFF);
                    word[size - 1] = 0xFE; // max - 1
                }
                4 => {
                    word[..size].fill(0xFF);
                    word[size - 1] = 0xFD; // max - 2
                }
                _ => {
                    word[..size].fill(0xFF);
                    word[size - 1] = 0xFC; // max - 3
                }
            }
            return FixedBytes::from(word);
        }

        let mut word = [0u8; 32];
        self.rng.fill(&mut word);
        FixedBytes::from(word)
    }

    /// Generate a random Ethereum address.
    fn address(&mut self) -> Address {
        let group = self.literals.addresses().to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group) {
                return val;
            }
        } else if roll < 50 {
            let mut bytes = [0u8; 20];
            match self.rng.u32(0..6) {
                0 => {}                // all zeros
                1 => bytes[0] = 1,     // 1 in first byte
                2 => bytes.fill(0xFF), // max
                3 => {
                    bytes.fill(0xFF);
                    bytes[19] = 0xFE; // max - 1
                }
                4 => {
                    bytes.fill(0xFF);
                    bytes[19] = 0xFD; // max - 2
                }
                _ => {
                    bytes.fill(0xFF);
                    bytes[19] = 0xFC; // max - 3
                }
            }
            return Address::from_slice(&bytes);
        }

        let mut bytes = [0u8; 20];
        self.rng.fill(&mut bytes);
        Address::from_slice(&bytes)
    }

    /// Generate random dynamic bytes.
    fn bytes(&mut self) -> Bytes {
        let group = self.literals.bytes().to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group) {
                return val;
            }
        } else if roll < 50 {
            let bytes = match self.rng.u32(0..6) {
                0 => vec![],         // empty
                1 => vec![0x01],     // 1 byte
                2 => vec![0xFF],     // max single byte
                3 => vec![0xFF; 32], // 32 bytes
                4 => vec![0xFF; 63], // 63 bytes
                _ => vec![0xFF; 64], // 64 bytes (max)
            };
            return Bytes::from(bytes);
        }

        let len = self.rng.usize(0..=64);
        let mut bytes = vec![0u8; len];
        self.rng.fill(&mut bytes);
        Bytes::from(bytes)
    }

    /// Generate a random string.
    fn string(&mut self) -> String {
        let group = self.literals.strings().to_vec();

        let roll = self.rng.u32(0..100);
        if roll < 20 {
            if let Some(val) = self.pick(&group) {
                return val;
            }
        } else if roll < 50 {
            let edge = match self.rng.u32(0..6) {
                0 => "".into(),
                1 => "a".into(),
                2 => " ".repeat(32),  // whitespace edge case
                3 => "\0".repeat(32), // null byte edge case
                4 => "a".repeat(32),  // max length
                _ => "a".repeat(31),  // max - 1
            };
            return edge;
        }

        let len = self.rng.usize(0..=32);
        (0..len).map(|_| self.rng.alphabetic()).collect()
    }

    /// Pick a random item from a slice, or return `None` if empty.
    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            None
        } else {
            Some(items[self.rng.usize(0..items.len())].clone())
        }
    }
}

/// Compute the maximum value for an unsigned integer of `bits` width.
fn max_for_bits(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    }
}

/// Compute the maximum positive value for a signed integer of `bits` width.
fn max_positive_for_bits(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        (U256::from(1) << 255) - U256::from(1)
    } else {
        (U256::from(1) << (bits - 1)) - U256::from(1)
    }
}

/// Compute the sign bit for a signed integer of `bits` width.
fn sign_bit(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::from(1) << 255
    } else {
        U256::from(1) << (bits - 1)
    }
}

/// Compute the mask for the low `bits` bits.
fn mask(bits: usize) -> U256 {
    if bits == 0 {
        U256::ZERO
    } else if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    }
}

/// Sign-extend a `bits`-wide raw unsigned value to a 256-bit signed integer.
fn sign_extend(raw: U256, bits: usize) -> I256 {
    if bits == 0 {
        return I256::ZERO;
    }

    let m = mask(bits);
    let value = raw & m;

    if bits >= 256 {
        return I256::from_raw(value);
    }

    let sb = sign_bit(bits);
    if value & sb == U256::ZERO {
        I256::from_raw(value)
    } else {
        let extended = value | (U256::MAX ^ m);
        I256::from_raw(extended)
    }
}
