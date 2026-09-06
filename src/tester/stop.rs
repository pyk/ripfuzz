//! Stop filters for test campaigns.
//!
//! [`StopOnRevert`] decides which reverted handler calls halt fuzzing. `Any`
//! matches every failure except an explicit `BrokenInvariantError` report,
//! which already has its own finding pipeline. `Selector` matches only
//! reverts whose output starts with the given 4-byte selector.
//!
//! [`StopOnPanic`] does the same for Solidity panics. `Any` matches every
//! `Panic` revert, while `Code` matches only one panic code such as `0x01`
//! for failed assertions.
//!
//! ```rust,no_run
//! use ripfuzz::tester::StopOnRevert;
//!
//! # let result: ripfuzz::evm::TransactionResult = todo!();
//! let filter: StopOnRevert = "0xaa9a98df".parse().unwrap();
//! if filter.matches(&result) {
//!     println!("stop: {}", filter.finding_id(&result));
//! }
//! ```

use std::fmt;
use std::str::FromStr;

use alloy_primitives::U256;
use alloy_sol_types::SolError;

use crate::evm::TransactionResult;

alloy_sol_types::sol! {
    error Panic(uint256 code);
}

/// Prefix of every stop-on-revert finding id.
pub const REVERT_PREFIX: &str = "REVERT:";

/// Prefix of every stop-on-panic finding id.
pub const PANIC_PREFIX: &str = "PANIC:";

/// Which reverts halt a test campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOnRevert {
    /// Halt on any reverted call, except `BrokenInvariantError` reports.
    Any,
    /// Halt only on reverts starting with this 4-byte selector.
    Selector([u8; 4]),
}

impl StopOnRevert {
    /// Whether the transaction result halts the campaign.
    pub fn matches(&self, result: &TransactionResult) -> bool {
        // 1. Successful calls never halt.
        if result.success {
            return false;
        }
        match self {
            // 2. Any failure halts, except explicit broken invariant reports
            //    which already travel through their own finding pipeline.
            Self::Any => result.broken_invariant().is_none(),
            // 3. A selector halts only when the revert output starts with it.
            Self::Selector(selector) => match result.output.as_ref() {
                Some(output) => output.len() >= 4 && output[..4] == selector[..],
                None => false,
            },
        }
    }

    /// The finding id recorded for a matched result.
    ///
    /// The id carries the revert identity so shrinking can preserve the exact
    /// revert:
    ///
    /// - revert output with at least 4 bytes: the `0x` selector
    /// - empty revert output: `empty`
    /// - halt without output: `halt`
    pub fn finding_id(&self, result: &TransactionResult) -> String {
        match result.output.as_ref() {
            // 1. Name revert output by its 4-byte selector.
            Some(output) if output.len() >= 4 => {
                format!("{REVERT_PREFIX}0x{}", hex::encode(&output[..4]))
            }
            // 2. Name empty revert output without a selector.
            Some(output) if output.is_empty() => format!("{REVERT_PREFIX}empty"),
            // 3. Name short output by its full bytes.
            Some(output) => format!("{REVERT_PREFIX}0x{}", hex::encode(output)),
            // 4. Name halts without output.
            None => format!("{REVERT_PREFIX}halt"),
        }
    }

    /// Whether the broken invariant id is a stop-on-revert finding.
    pub fn is_revert_finding(id: &str) -> bool {
        id.starts_with(REVERT_PREFIX)
    }

    /// Whether replaying a sequence still reproduces the expected revert id.
    pub fn matches_expected(result: &TransactionResult, expected_id: &str) -> bool {
        // 1. Require the revert prefix and a failing result.
        let Some(suffix) = expected_id.strip_prefix(REVERT_PREFIX) else {
            return false;
        };
        if result.success {
            return false;
        }

        // 2. Match halts and empty reverts by their marker.
        if suffix == "halt" {
            return result.output.is_none();
        }
        if suffix == "empty" {
            return matches!(result.output.as_ref(), Some(output) if output.is_empty());
        }

        // 3. Match selector ids against the revert output prefix.
        let Some(selector_hex) = suffix.strip_prefix("0x") else {
            return false;
        };
        let Ok(selector) = hex::decode(selector_hex) else {
            return false;
        };
        match result.output.as_ref() {
            Some(output) if selector.len() == 4 => output.len() >= 4 && output[..4] == selector[..],
            Some(output) => output.as_ref() == selector.as_slice(),
            None => false,
        }
    }
}

impl fmt::Display for StopOnRevert {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(formatter, "any"),
            Self::Selector(selector) => write!(formatter, "0x{}", hex::encode(selector)),
        }
    }
}

impl FromStr for StopOnRevert {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        // 1. Accept the wildcard spellings for any revert.
        if input == "any" || input == "*" {
            return Ok(Self::Any);
        }

        // 2. Require a `0x`-prefixed 4-byte selector otherwise.
        let error = || {
            format!(
                "stop on revert must be `any` or a `0x`-prefixed 4-byte selector like `0xaa9a98df`, got `{input}`"
            )
        };
        let digits = input.strip_prefix("0x").ok_or_else(error)?;
        if digits.len() != 8 {
            return Err(error());
        }
        let decoded = hex::decode(digits).map_err(|_| error())?;
        let selector: [u8; 4] = decoded.try_into().map_err(|_| error())?;
        Ok(Self::Selector(selector))
    }
}

/// Which Solidity panics halt a test campaign.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOnPanic {
    /// Halt on any `Panic` revert.
    Any,
    /// Halt only on panics with this code, e.g. `0x01` for assertions.
    Code(U256),
}

impl StopOnPanic {
    /// Whether the transaction result halts the campaign.
    pub fn matches(&self, result: &TransactionResult) -> bool {
        // 1. Successful calls never halt.
        if result.success {
            return false;
        }

        // 2. Require a decodable `Panic` revert.
        let Some(output) = result.output.as_ref() else {
            return false;
        };
        let Ok(panic) = Panic::abi_decode(output) else {
            return false;
        };
        match self {
            // 3a. Any panic code halts.
            Self::Any => true,
            // 3b. A code halts only on that panic code.
            Self::Code(code) => panic.code == *code,
        }
    }

    /// The finding id recorded for a matched result.
    ///
    /// The id carries the panic code so shrinking preserves it:
    ///
    /// - decodable panics: the code as `0x` hex, e.g. `0x01`
    /// - anything else: `unknown`
    pub fn finding_id(&self, result: &TransactionResult) -> String {
        // 1. Name decodable panics by their code.
        if let Some(output) = result.output.as_ref()
            && let Ok(panic) = Panic::abi_decode(output)
        {
            return format!("{PANIC_PREFIX}0x{:02x}", panic.code);
        }

        // 2. Name anything else without a code.
        format!("{PANIC_PREFIX}unknown")
    }

    /// Whether the broken invariant id is a stop-on-panic finding.
    pub fn is_panic_finding(id: &str) -> bool {
        id.starts_with(PANIC_PREFIX)
    }

    /// Whether replaying a sequence still reproduces the expected panic id.
    pub fn matches_expected(result: &TransactionResult, expected_id: &str) -> bool {
        // 1. Require the panic prefix and a failing result.
        let Some(suffix) = expected_id.strip_prefix(PANIC_PREFIX) else {
            return false;
        };
        if result.success {
            return false;
        }

        // 2. Unknown panics reproduce on any panic revert.
        if suffix == "unknown" {
            return match result.output.as_ref() {
                Some(output) => Panic::abi_decode(output).is_ok(),
                None => false,
            };
        }

        // 3. Match coded panics against the decoded panic code.
        let Some(code_hex) = suffix.strip_prefix("0x") else {
            return false;
        };
        let Ok(expected) = U256::from_str_radix(code_hex, 16) else {
            return false;
        };
        match result.output.as_ref() {
            Some(output) => match Panic::abi_decode(output) {
                Ok(panic) => panic.code == expected,
                Err(_) => false,
            },
            None => false,
        }
    }
}

impl fmt::Display for StopOnPanic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => write!(formatter, "any"),
            Self::Code(code) => write!(formatter, "0x{code:02x}"),
        }
    }
}

impl FromStr for StopOnPanic {
    type Err = String;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        // 1. Accept the wildcard spellings for any panic.
        if input == "any" || input == "*" {
            return Ok(Self::Any);
        }

        // 2. Accept decimal or `0x`-prefixed panic codes otherwise.
        let error = || {
            format!("stop on panic must be `any` or a panic code like `0x01` or `1`, got `{input}`")
        };
        let code = match input.strip_prefix("0x") {
            Some(digits) => U256::from_str_radix(digits, 16).map_err(|_| error())?,
            None => input.parse::<U256>().map_err(|_| error())?,
        };
        Ok(Self::Code(code))
    }
}

#[cfg(test)]
mod tests {
    use alloy_sol_types::SolValue as _;
    use revm::primitives::Bytes;

    use super::*;
    use crate::evm::forkdb::RpcStats;

    alloy_sol_types::sol! {
        error BrokenInvariantError(string id, string description);
        error CustomRevert();
        error Error(string message);
    }

    fn success() -> TransactionResult {
        TransactionResult {
            success: true,
            gas_used: 0,
            output: None,
            logs: Vec::new(),
            created_address: None,
            elapsed: std::time::Duration::ZERO,
            rpc: RpcStats::default(),
        }
    }

    fn revert(output: Option<Vec<u8>>) -> TransactionResult {
        TransactionResult {
            success: false,
            gas_used: 0,
            output: output.map(Bytes::from),
            logs: Vec::new(),
            created_address: None,
            elapsed: std::time::Duration::ZERO,
            rpc: RpcStats::default(),
        }
    }

    fn broken_output() -> Vec<u8> {
        let mut encoded = BrokenInvariantError::SELECTOR.to_vec();
        encoded.extend(("ID-1", "bad").abi_encode_params());
        encoded
    }

    fn custom_output() -> Vec<u8> {
        CustomRevert::SELECTOR.to_vec()
    }

    #[test]
    fn parses_any_and_wildcard() {
        assert_eq!("any".parse::<StopOnRevert>().unwrap(), StopOnRevert::Any);
        assert_eq!("*".parse::<StopOnRevert>().unwrap(), StopOnRevert::Any);
    }

    #[test]
    fn parses_selector() {
        assert_eq!(
            "0xaa9a98df".parse::<StopOnRevert>().unwrap(),
            StopOnRevert::Selector([0xaa, 0x9a, 0x98, 0xdf])
        );
    }

    #[test]
    fn rejects_invalid_selector() {
        assert_eq!(
            "nope".parse::<StopOnRevert>().unwrap_err(),
            "stop on revert must be `any` or a `0x`-prefixed 4-byte selector like `0xaa9a98df`, got `nope`"
        );
        assert_eq!(
            "0x123".parse::<StopOnRevert>().unwrap_err(),
            "stop on revert must be `any` or a `0x`-prefixed 4-byte selector like `0xaa9a98df`, got `0x123`"
        );
        assert_eq!(
            "0xzzzzzzzz".parse::<StopOnRevert>().unwrap_err(),
            "stop on revert must be `any` or a `0x`-prefixed 4-byte selector like `0xaa9a98df`, got `0xzzzzzzzz`"
        );
    }

    #[test]
    fn any_matches_reverts_but_not_success_or_broken_reports() {
        assert!(!StopOnRevert::Any.matches(&success()));
        assert!(StopOnRevert::Any.matches(&revert(Some(custom_output()))));
        assert!(StopOnRevert::Any.matches(&revert(Some(Vec::new()))));
        assert!(StopOnRevert::Any.matches(&revert(None)));
        assert!(!StopOnRevert::Any.matches(&revert(Some(broken_output()))));
    }

    #[test]
    fn selector_matches_only_its_prefix() {
        let filter = StopOnRevert::Selector(CustomRevert::SELECTOR);
        assert!(!filter.matches(&success()));
        assert!(filter.matches(&revert(Some(custom_output()))));
        assert!(!filter.matches(&revert(Some(broken_output()))));
        assert!(!filter.matches(&revert(Some(Vec::new()))));
        assert!(!filter.matches(&revert(None)));
    }

    #[test]
    fn finding_id_names_selector_empty_and_halt() {
        let filter = StopOnRevert::Any;
        let selector = format!("0x{}", hex::encode(CustomRevert::SELECTOR));
        assert_eq!(
            filter.finding_id(&revert(Some(custom_output()))),
            format!("{REVERT_PREFIX}{selector}")
        );
        assert_eq!(
            filter.finding_id(&revert(Some(Vec::new()))),
            format!("{REVERT_PREFIX}empty")
        );
        assert_eq!(
            filter.finding_id(&revert(None)),
            format!("{REVERT_PREFIX}halt")
        );
    }

    #[test]
    fn matches_expected_reproduces_the_recorded_revert() {
        let selector = format!("0x{}", hex::encode(CustomRevert::SELECTOR));
        let id = format!("{REVERT_PREFIX}{selector}");
        assert!(StopOnRevert::matches_expected(
            &revert(Some(custom_output())),
            &id
        ));
        assert!(!StopOnRevert::matches_expected(
            &revert(Some(broken_output())),
            &id
        ));
        assert!(!StopOnRevert::matches_expected(&success(), &id));
        assert!(!StopOnRevert::matches_expected(
            &revert(Some(custom_output())),
            "ID-1"
        ));
        assert!(StopOnRevert::matches_expected(
            &revert(None),
            &format!("{REVERT_PREFIX}halt")
        ));
        assert!(StopOnRevert::matches_expected(
            &revert(Some(Vec::new())),
            &format!("{REVERT_PREFIX}empty")
        ));
    }

    #[test]
    fn displays_any_and_selector() {
        assert_eq!(StopOnRevert::Any.to_string(), "any");
        assert_eq!(
            StopOnRevert::Selector([0xaa, 0x9a, 0x98, 0xdf]).to_string(),
            "0xaa9a98df"
        );
    }

    fn panic_output(code: u64) -> Vec<u8> {
        let mut encoded = Panic::SELECTOR.to_vec();
        encoded.extend((U256::from(code),).abi_encode_params());
        encoded
    }

    #[test]
    fn parses_any_and_panic_codes() {
        assert_eq!("any".parse::<StopOnPanic>().unwrap(), StopOnPanic::Any);
        assert_eq!("*".parse::<StopOnPanic>().unwrap(), StopOnPanic::Any);
        assert_eq!(
            "0x01".parse::<StopOnPanic>().unwrap(),
            StopOnPanic::Code(U256::from(1))
        );
        assert_eq!(
            "1".parse::<StopOnPanic>().unwrap(),
            StopOnPanic::Code(U256::from(1))
        );
    }

    #[test]
    fn rejects_invalid_panic_code() {
        assert_eq!(
            "nope".parse::<StopOnPanic>().unwrap_err(),
            "stop on panic must be `any` or a panic code like `0x01` or `1`, got `nope`"
        );
        assert_eq!(
            "0xzz".parse::<StopOnPanic>().unwrap_err(),
            "stop on panic must be `any` or a panic code like `0x01` or `1`, got `0xzz`"
        );
    }

    #[test]
    fn any_panic_matches_panics_but_not_success_or_other_reverts() {
        assert!(!StopOnPanic::Any.matches(&success()));
        assert!(StopOnPanic::Any.matches(&revert(Some(panic_output(1)))));
        assert!(StopOnPanic::Any.matches(&revert(Some(panic_output(17)))));
        assert!(!StopOnPanic::Any.matches(&revert(Some(custom_output()))));
        assert!(!StopOnPanic::Any.matches(&revert(Some(broken_output()))));
        assert!(!StopOnPanic::Any.matches(&revert(Some(Vec::new()))));
        assert!(!StopOnPanic::Any.matches(&revert(None)));
    }

    #[test]
    fn panic_code_matches_only_its_code() {
        let filter = StopOnPanic::Code(U256::from(1));
        assert!(!filter.matches(&success()));
        assert!(filter.matches(&revert(Some(panic_output(1)))));
        assert!(!filter.matches(&revert(Some(panic_output(17)))));
        assert!(!filter.matches(&revert(Some(custom_output()))));
    }

    #[test]
    fn panic_finding_id_names_the_code() {
        let filter = StopOnPanic::Any;
        assert_eq!(
            filter.finding_id(&revert(Some(panic_output(1)))),
            format!("{PANIC_PREFIX}0x01")
        );
        assert_eq!(
            filter.finding_id(&revert(Some(panic_output(17)))),
            format!("{PANIC_PREFIX}0x11")
        );
    }

    #[test]
    fn matches_expected_reproduces_the_recorded_panic() {
        let id = format!("{PANIC_PREFIX}0x01");
        assert!(StopOnPanic::matches_expected(
            &revert(Some(panic_output(1))),
            &id
        ));
        assert!(!StopOnPanic::matches_expected(
            &revert(Some(panic_output(17))),
            &id
        ));
        assert!(!StopOnPanic::matches_expected(&success(), &id));
        assert!(!StopOnPanic::matches_expected(
            &revert(Some(custom_output())),
            &id
        ));
        assert!(!StopOnPanic::matches_expected(
            &revert(Some(panic_output(1))),
            "ID-1"
        ));
    }

    #[test]
    fn displays_any_and_panic_code() {
        assert_eq!(StopOnPanic::Any.to_string(), "any");
        assert_eq!(StopOnPanic::Code(U256::from(1)).to_string(), "0x01");
    }
}
