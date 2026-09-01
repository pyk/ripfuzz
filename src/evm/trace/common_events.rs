//! Standard events from widely-deployed contracts.
//!
//! [`CommonEvents::abi`] is consumed by [`TraceContext`](super::TraceContext)
//! as a last-priority ABI so logs from forked or external contracts still
//! decode when no project artifact declares them.

use alloy_json_abi::JsonAbi;
use alloy_sol_types::sol;

sol! {
    #[sol(abi)]
    contract StandardEvents {
        event Transfer(address indexed from, address indexed to, uint256 value);
        event Approval(address indexed owner, address indexed spender, uint256 value);
        event ApprovalForAll(address indexed owner, address indexed operator, bool approved);
        event Deposit(address indexed dst, uint256 wad);
        event Withdrawal(address indexed src, uint256 wad);
        event OwnershipTransferred(address indexed previousOwner, address indexed newOwner);
    }
}

/// ERC20, ERC721, WETH9, and Ownable events used as a decoding fallback.
pub struct CommonEvents;

impl CommonEvents {
    /// Standard-event ABI used as a decoding fallback.
    pub fn abi() -> JsonAbi {
        StandardEvents::abi::contract()
    }
}
