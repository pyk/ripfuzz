//! Typed errors for chain_v2 operations.

use revm::primitives::Bytes;

/// Error during contract deployment.
#[derive(Debug)]
pub enum DeployError {
    Reverted { reason: String, output: Bytes },
    Halt { reason: String },
    NoAddress,
    Other(anyhow::Error),
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reverted { reason, .. } => write!(f, "deployment reverted: {reason}"),
            Self::Halt { reason } => write!(f, "deployment halted: {reason}"),
            Self::NoAddress => write!(f, "deployment succeeded but returned no address"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DeployError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for DeployError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Error during setup execution.
#[derive(Debug)]
pub enum SetupError {
    Reverted { reason: String, output: Bytes },
    Halt { reason: String },
    Other(anyhow::Error),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reverted { reason, .. } => write!(f, "setup reverted: {reason}"),
            Self::Halt { reason } => write!(f, "setup halted: {reason}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for SetupError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Solidity `Error(string)` selector: `keccak256("Error(string)")[:4]`
const ERROR_SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];

/// Decode a Solidity `Error(string)` revert payload.
pub fn decode_solidity_error(output: &Bytes) -> Option<String> {
    if output.len() < 4 || output[..4] != ERROR_SELECTOR {
        return None;
    }
    let string_type = alloy_dyn_abi::DynSolType::String;
    let decoded = crate::result_to_option(string_type.abi_decode_params(&output[4..]))?;
    match decoded {
        alloy_dyn_abi::DynSolValue::String(s) => Some(s),
        _ => None,
    }
}
