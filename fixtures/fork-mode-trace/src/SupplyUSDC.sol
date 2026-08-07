// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "./RVM.sol";

interface IERC20 {
    function approve(address spender, uint256 amount) external returns (bool);
}

interface IPool {
    function supply(address asset, uint256 amount, address onBehalfOf, uint16 referralCode) external;
}

/// @notice Harness that supplies USDC to the Aave V3 pool on Base.
contract SupplyUSDC {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    // Base USDC token address.
    address constant USDC = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;

    // Aave V3 Pool Proxy on Base.
    address constant POOL = 0xA238Dd80C259a72e81d7e4664a9801593F98d1c5;

    /// @notice Supply USDC to the Aave V3 pool.
    function supply() external {
        uint256 amount = 10 * 1e6; // 10 USDC (6 decimals)

        // Label the harness contract for readable trace output.
        rvm.label(address(this), "SupplyUSDC");

        // -----------------------------------------------------------------
        // Set USDC balance for the harness contract.
        //
        // USDC uses slot 9 for the _balances mapping.
        // slot = keccak256(abi.encode(address(this), uint256(9)))
        // -----------------------------------------------------------------
        bytes32 balanceSlot = keccak256(abi.encode(address(this), uint256(9)));
        rvm.store(USDC, balanceSlot, bytes32(amount));

        // Approve USDC spend by the pool.
        IERC20(USDC).approve(POOL, amount);

        // Supply USDC to the pool.
        IPool(POOL).supply(USDC, amount, address(this), 0);
    }
}
