// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

interface IERC20 {
    function balanceOf(address) external view returns (uint256);
    function totalSupply() external view returns (uint256);
}

/// @notice Example harness that forks mainnet via `rvm.fork` then reads USDC.
contract ForkHarness {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    IERC20 constant usdc = IERC20(0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48);
    uint256 public recordedBalance;

    function setup() external {
        rvm.fork(rvm.getEnv("ETH_RPC_URL"), 21_000_000);
        recordedBalance = usdc.balanceOf(address(usdc));
    }

    function call_read_usdc() external {
        recordedBalance = usdc.totalSupply();
    }

    function invariant_usdc_has_supply() external view {
        assert(usdc.totalSupply() > 0);
    }
}
