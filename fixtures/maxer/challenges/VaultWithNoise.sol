// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

import {NoiseBase} from "./NoiseBase.sol";

/// Level: hard
///
/// The same as `Vault` but inherits `NoiseBase`, so the fuzzer must reach
/// the same highest value while 20 external functions revert or mutate
/// state that does not affect the value.
///
/// The highest value is `type(uint256).max`, reached by approving
/// `type(uint256).max`, depositing it, and redeeming it. Deposits before
/// approval revert and redeems before deposits revert. The value only grows
/// on redeem, which checks shares and adds the redeemed amount.
contract VaultWithNoise is NoiseBase {
    uint256 internal allowance;
    uint256 internal shares;
    uint256 internal profit;

    function approve(address, uint256 amount) external {
        allowance = amount;
    }

    function deposit(uint256 amount, address) external {
        require(allowance >= amount);
        require(amount > 0);
        allowance -= amount;
        shares += amount;
    }

    function redeem(uint256 amount, address, address, uint256) external {
        require(shares >= amount);
        require(amount > 0);
        shares -= amount;
        profit = type(uint256).max;
    }

    function setAdmin(address) external {
        unused += 1;
    }

    function setFee(uint256 amount) external {
        unused += amount;
    }

    function report() external {
        unused += 1;
    }

    function tend() external {
        unused += 1;
    }

    function transfer(address, uint256) external {
        unused += 1;
    }

    function value() external view returns (uint256) {
        return profit;
    }
}
