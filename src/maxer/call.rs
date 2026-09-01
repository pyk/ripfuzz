//! Single harness calls with generated arguments.
//!
//! [`Call`] pairs a handler function with randomly generated arguments and
//! encodes them into EVM calldata.
//!
//! ```rust
//! use alloy_json_abi::Function;
//! use ripfuzz::maxer::Call;
//!
//! // let function = Function::parse("deposit(uint256)")?;
//! // let call = Call::random(&mut rng, &function)?;
//! // let data = call.calldata()?;
//! ```

use alloy_dyn_abi::{DynSolType, DynSolValue, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::{Address, FixedBytes, I256, U256};
use anyhow::{Context, Result, bail};
use revm::primitives::Bytes;

use crate::evm::Transaction;

/// Maximum length of randomly generated dynamic bytes.
const MAX_BYTES_LEN: usize = 32;

/// Maximum length of randomly generated dynamic strings.
const MAX_STRING_LEN: usize = 16;

/// Maximum length of randomly generated dynamic arrays.
const MAX_ARRAY_LEN: usize = 4;

/// One in this many uniform integers becomes a boundary value instead, because
/// a uniform random value almost never lands on a Solidity edge case.
const BOUNDARY_PROBABILITY: u32 = 8;

/// Number of boundary candidates to pick from.
const BOUNDARY_COUNT: usize = 6;

/// A single harness call with generated arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    function: Function,
    args: DynSolValue,
}

impl Call {
    /// Create a call from a function and its tuple of arguments.
    pub fn new(function: Function, args: DynSolValue) -> Self {
        Self { function, args }
    }

    /// Generate a random call for the given function.
    pub fn random(rng: &mut fastrand::Rng, function: &Function) -> Result<Self> {
        let mut inputs = Vec::with_capacity(function.inputs.len());
        for input in &function.inputs {
            // Resolve the ABI parameter instead of parsing `input.ty` alone,
            // because struct parameters carry their fields in `components`
            // and the bare type string is just `tuple`.
            let ty = input
                .resolve()
                .with_context(|| format!("failed to resolve type `{}`", input.ty))?;
            inputs.push(random_value(rng, &ty)?);
        }
        Ok(Self::new(function.clone(), DynSolValue::Tuple(inputs)))
    }

    /// The human-readable signature of the called function.
    pub fn signature(&self) -> String {
        self.function.signature()
    }

    /// The called function.
    pub fn function(&self) -> &Function {
        &self.function
    }

    /// Encode the call as EVM calldata: selector plus encoded arguments.
    pub fn calldata(&self) -> Bytes {
        let args = self.args.abi_encode_params();
        let mut buf = Vec::with_capacity(4 + args.len());
        buf.extend_from_slice(self.function.selector().as_slice());
        buf.extend_from_slice(&args);
        Bytes::from(buf)
    }

    /// Build the transaction for this call against the target.
    pub fn transaction(&self, target: Address, caller: Address) -> Transaction {
        Transaction::new(target)
            .caller(caller)
            .calldata(self.calldata())
    }
}

/// Generate a random value for the given ABI type.
fn random_value(rng: &mut fastrand::Rng, ty: &DynSolType) -> Result<DynSolValue> {
    Ok(match ty {
        DynSolType::Bool => DynSolValue::Bool(rng.bool()),
        DynSolType::Int(bits) => DynSolValue::Int(I256::from_raw(random_uint(rng, 256)), *bits),
        DynSolType::Uint(bits) => DynSolValue::Uint(random_uint(rng, *bits), *bits),
        DynSolType::FixedBytes(size) => {
            let mut word = FixedBytes::<32>::new([0; 32]);
            rng.fill(&mut word.as_mut_slice()[..*size]);
            DynSolValue::FixedBytes(word, *size)
        }
        DynSolType::Address => {
            let mut bytes = [0; 20];
            rng.fill(&mut bytes);
            DynSolValue::Address(Address::new(bytes))
        }
        DynSolType::Function => {
            bail!("random arguments for `function` parameters are not supported")
        }
        DynSolType::Bytes => {
            let mut bytes = vec![0; rng.usize(..=MAX_BYTES_LEN)];
            rng.fill(&mut bytes);
            DynSolValue::Bytes(bytes)
        }
        DynSolType::String => {
            let string: String = (0..rng.usize(..=MAX_STRING_LEN))
                .map(|_| rng.alphanumeric())
                .collect();
            DynSolValue::String(string)
        }
        DynSolType::Array(ty) => {
            let len = rng.usize(..=MAX_ARRAY_LEN);
            let values = random_values(rng, ty, len)?;
            DynSolValue::Array(values)
        }
        DynSolType::FixedArray(ty, len) => {
            let values = random_values(rng, ty, *len)?;
            DynSolValue::FixedArray(values)
        }
        DynSolType::Tuple(types) => {
            let values = types
                .iter()
                .map(|ty| random_value(rng, ty))
                .collect::<Result<Vec<DynSolValue>>>()?;
            DynSolValue::Tuple(values)
        }
    })
}

/// Generate `len` random values of the given ABI type.
fn random_values(rng: &mut fastrand::Rng, ty: &DynSolType, len: usize) -> Result<Vec<DynSolValue>> {
    (0..len).map(|_| random_value(rng, ty)).collect()
}

/// Generate a random `uint256` limited to the given bit width.
fn random_uint(rng: &mut fastrand::Rng, bits: usize) -> U256 {
    if rng.u32(..BOUNDARY_PROBABILITY) == 0 {
        return boundary_uint(rng, bits);
    }
    let low = U256::from(rng.u128(..));
    let high = U256::from(rng.u128(..)) << 128;
    let value = high | low;
    if bits >= 256 {
        value
    } else {
        value & ((U256::from(1) << bits) - U256::from(1))
    }
}

/// Generate a boundary `uint256` limited to the given bit width.
fn boundary_uint(rng: &mut fastrand::Rng, bits: usize) -> U256 {
    let max = if bits >= 256 {
        U256::MAX
    } else {
        (U256::from(1) << bits) - U256::from(1)
    };
    let candidates = [
        U256::ZERO,
        U256::from(1),
        U256::from(2),
        max,
        max >> 1,
        U256::from(10_000_000_000_000_000_000u64),
    ];
    candidates[rng.usize(..BOUNDARY_COUNT)] & max
}

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::DynSolValue;
    use alloy_json_abi::Function;
    use alloy_primitives::U256;

    use super::*;

    #[test]
    fn calldata_starts_with_selector() {
        let function = Function::parse("set(uint256)").unwrap();
        let call = Call::new(
            function.clone(),
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(1), 256)]),
        );
        let data = call.calldata();

        assert_eq!(data.len(), 36);
        assert_eq!(&data[..4], function.selector().as_slice());
    }

    #[test]
    fn random_call_matches_inputs() {
        let function = Function::parse("set(uint256,address,bool)").unwrap();
        let call = Call::random(&mut fastrand::Rng::new(), &function).unwrap();

        let DynSolValue::Tuple(args) = call.args else {
            panic!("arguments must be a tuple");
        };
        assert_eq!(args.len(), 3);
        assert!(matches!(args[0], DynSolValue::Uint(_, 256)));
        assert!(matches!(args[1], DynSolValue::Address(_)));
        assert!(matches!(args[2], DynSolValue::Bool(_)));
    }

    #[test]
    fn random_call_encodes() {
        let function = Function::parse("f(bytes,string,uint256[2])").unwrap();
        let call = Call::random(&mut fastrand::Rng::new(), &function).unwrap();
        let data = call.calldata();

        assert!(!data.is_empty());
        assert_eq!(&data[..4], function.selector().as_slice());
    }

    /// A struct parameter must resolve through its ABI `components`, because
    /// the bare JSON-ABI type string is just `tuple` and cannot be parsed on
    /// its own. Regression test for campaigns that crashed at startup when a
    /// handler took a struct argument.
    #[test]
    fn random_call_supports_struct_parameters() {
        let function = Function::parse("f((uint256,address),uint256[])").unwrap();
        let call = Call::random(&mut fastrand::Rng::new(), &function).unwrap();

        let DynSolValue::Tuple(args) = &call.args else {
            panic!("arguments must be a tuple");
        };
        assert_eq!(args.len(), 2);
        let DynSolValue::Tuple(struct_arg) = &args[0] else {
            panic!("first argument must be a tuple");
        };
        assert_eq!(struct_arg.len(), 2);
        assert!(matches!(struct_arg[0], DynSolValue::Uint(_, 256)));
        assert!(matches!(struct_arg[1], DynSolValue::Address(_)));
        assert!(matches!(args[1], DynSolValue::Array(_)));

        let data = call.calldata();
        assert_eq!(&data[..4], function.selector().as_slice());
    }

    #[test]
    fn unsupported_function_parameter_fails() {
        let function = Function::parse("f(function)").unwrap();
        let err = Call::random(&mut fastrand::Rng::new(), &function).unwrap_err();

        assert_eq!(
            err.to_string(),
            "random arguments for `function` parameters are not supported"
        );
    }

    #[test]
    fn random_uints_stay_within_the_bit_width() {
        let mut rng = fastrand::Rng::new();
        for bits in [1, 4, 8, 64, 255, 256] {
            let mask = if bits >= 256 {
                U256::MAX
            } else {
                (U256::from(1) << bits) - U256::from(1)
            };
            for _ in 0..200 {
                let value = random_uint(&mut rng, bits);
                assert!(value <= mask, "value {value} exceeds {bits} bits");
            }
        }
    }

    #[test]
    fn function_exposes_the_called_function() {
        let function = Function::parse("set(uint256)").unwrap();
        let call = Call::new(
            function.clone(),
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(1), 256)]),
        );

        assert_eq!(call.function().selector(), function.selector());
    }

    #[test]
    fn transaction_targets_the_harness() {
        let function = Function::parse("set(uint256)").unwrap();
        let call = Call::new(
            function,
            DynSolValue::Tuple(vec![DynSolValue::Uint(U256::from(1), 256)]),
        );
        let target = Address::new([7; 20]);
        let caller = Address::new([9; 20]);
        let transaction = call.transaction(target, caller);

        assert_eq!(transaction.target, target);
        assert_eq!(transaction.caller, caller);
        assert_eq!(transaction.calldata, call.calldata());
    }
}
