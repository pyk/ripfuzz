//! Single harness calls with generated arguments.
//!
//! [`Call`] pairs a handler function with randomly generated arguments,
//! seeded with the literals extracted from the harness sources, and encodes
//! them into EVM calldata.
//!
//! ```rust,no_run
//! use alloy_json_abi::Function;
//! use fastrand::Rng;
//! use ripfuzz::tester::{Call, LiteralExtractor};
//!
//! # fn main() -> anyhow::Result<()> {
//! # let mut rng = Rng::new();
//! # let function = Function::parse("deposit(uint256)").unwrap();
//! # let literals = LiteralExtractor::new();
//! let call = Call::random(&mut rng, &function, &literals)?;
//! let data = call.calldata();
//! println!("{} bytes of calldata", data.len());
//! # Ok(())
//! # }
//! ```

use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::Function;
use alloy_primitives::Address;
use anyhow::{Context, Result};
use fastrand::Rng;
use revm::primitives::Bytes;

use crate::evm::Transaction;
use crate::tester::corpus::literal::LiteralExtractor;
use crate::tester::corpus::rvg::RandomValueGenerator;

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

    /// Generate a random call for the given function, seeded with the
    /// extracted literals.
    pub fn random(rng: &mut Rng, function: &Function, literals: &LiteralExtractor) -> Result<Self> {
        let mut generator = RandomValueGenerator::new(rng, literals);
        let mut inputs = Vec::with_capacity(function.inputs.len());
        for input in &function.inputs {
            // Resolve the ABI parameter instead of parsing `input.ty` alone,
            // because struct parameters carry their fields in `components`
            // and the bare type string is just `tuple`.
            let ty = input
                .resolve()
                .with_context(|| format!("failed to resolve type `{}`", input.ty))?;
            inputs.push(generator.value(&ty));
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

    /// Encode this call as EVM calldata: selector plus encoded arguments.
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

#[cfg(test)]
mod tests {
    use alloy_dyn_abi::{DynSolType, DynSolValue};
    use alloy_json_abi::Function;
    use alloy_primitives::{Address, U256};

    use super::*;

    fn literals() -> LiteralExtractor {
        LiteralExtractor::new()
    }

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
        let call = Call::random(&mut Rng::new(), &function, &literals()).unwrap();

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
        let call = Call::random(&mut Rng::new(), &function, &literals()).unwrap();
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
        let call = Call::random(&mut Rng::new(), &function, &literals()).unwrap();

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

    /// Generated integers must fit their declared bit width, because a value
    /// beyond it reverts with an ABI encoding error before the harness runs.
    #[test]
    fn random_uints_stay_within_the_bit_width() {
        let mut rng = Rng::new();
        for bits in [8, 64, 255, 256] {
            let ty = DynSolType::Uint(bits);
            for _ in 0..200 {
                let value = RandomValueGenerator::new(&mut rng, &literals()).value(&ty);
                let DynSolValue::Uint(value, width) = value else {
                    panic!("expected a uint value");
                };
                assert_eq!(width, bits);
                let mask = if bits >= 256 {
                    U256::MAX
                } else {
                    (U256::from(1) << bits) - U256::from(1)
                };
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
    }
}
