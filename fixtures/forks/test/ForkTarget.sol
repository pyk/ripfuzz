// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

interface IERC20 {
    function balanceOf(address) external view returns (uint256);
    function totalSupply() external view returns (uint256);
}

contract ForkTarget {
    IERC20 constant usdc = IERC20(0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48);
    uint256 public recordedBalance;

    function setUp() external {
        recordedBalance = usdc.balanceOf(address(usdc));
    }

    function call_read_usdc() external {
        recordedBalance = usdc.totalSupply();
    }

    function invariant_usdc_has_supply() external view {
        assert(usdc.totalSupply() > 0);
    }
}
