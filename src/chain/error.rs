//! Typed errors for chain operations.

use std::fmt;

/// Error during chain initialization (deployment).
#[derive(Debug)]
pub enum ChainInitError {
    DeploymentFailed { reason: String, trace: String },
    Other(anyhow::Error),
}

impl fmt::Display for ChainInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeploymentFailed { reason, trace } => {
                write!(f, "deployment failed: {reason}\n\nTrace:\n{trace}")
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ChainInitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for ChainInitError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Error during chain setup (`setup()` call).
#[derive(Debug)]
pub enum ChainSetupError {
    SetupFailed { reason: String, trace: String },
    Other(anyhow::Error),
}

impl fmt::Display for ChainSetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetupFailed { reason, trace } => {
                write!(f, "setup failed: {reason}\n\nTrace:\n{trace}")
            }
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ChainSetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Other(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<anyhow::Error> for ChainSetupError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
    }
}

/// Error during sequence execution.
#[derive(Debug)]
pub struct ChainExecutionError(pub anyhow::Error);

impl fmt::Display for ChainExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "execution failed: {}", self.0)
    }
}

impl std::error::Error for ChainExecutionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

impl From<anyhow::Error> for ChainExecutionError {
    fn from(e: anyhow::Error) -> Self {
        Self(e)
    }
}
