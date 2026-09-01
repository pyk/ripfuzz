// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {RVM} from "./RVM.sol";

interface IWETH {
    function decimals() external view returns (uint8);
    function balanceOf(address) external view returns (uint256);
}

/// @notice Regression fixture for verifying that fork mode correctly fetches
/// and caches remote contract account data. The contract interacts with the
/// mainnet WETH contract (0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2) in
/// four different execution paths -- constructor (decimals), setup
/// (balanceOf), handler function (balanceOf), and invariant (balanceOf) -- to
/// confirm that cached account and storage data is consistent and that no
/// redundant RPC calls occur after the initial fetches.
contract InteractWithWETH {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    /// Mainnet WETH at block 25_259_523.
    IWETH internal constant WETH = IWETH(0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2);

    /// vitalik.eth address.
    address internal constant VITALIK = 0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045;

    /// vitalik.eth WETH balance at block 25_259_523.
    uint256 internal constant EXPECTED = 1_461_898_164_019_088_870;

    constructor() {
        rvm.fork("mock://test", 25_259_523);
        assert(WETH.decimals() == 18);
    }

    function setup() external {
        uint256 bal = WETH.balanceOf(VITALIK);
        assert(bal == EXPECTED);
    }

    function checkBalance() external {
        uint256 bal = WETH.balanceOf(VITALIK);
        assert(bal == EXPECTED);
    }

    function invariant_checkBalance() external view {
        uint256 bal = WETH.balanceOf(VITALIK);
        assert(bal == EXPECTED);
    }
}
