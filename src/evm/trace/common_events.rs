//! Standard events from widely-deployed contracts.
//!
//! [`CommonEvents::abi`] is consumed by [`TraceContext`](super::TraceContext)
//! as a last-priority ABI so logs from forked or external contracts still
//! decode when no project artifact declares them.

use alloy_json_abi::JsonAbi;

/// ERC20, ERC721, WETH9, and Ownable events used as a decoding fallback.
pub struct CommonEvents;

impl CommonEvents {
    /// Parse the embedded standard-event ABI.
    pub fn abi() -> JsonAbi {
        serde_json::from_str(include_str!("common_events.json")).unwrap_or_default()
    }
}
