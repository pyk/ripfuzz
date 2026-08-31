// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Level: hard
///
/// The highest value is `type(uint256).max`, reached by approving
/// `type(uint256).max`, depositing it, and redeeming it. Deposits before
/// approval revert and redeems before deposits revert. The value only grows
/// on redeem, which checks shares and adds the redeemed amount.
contract Vault {
    uint256 internal allowance;
    uint256 internal shares;
    uint256 internal profit;
    uint256 internal unused;

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

    function donateETH(address, uint256) external {
        unused += 1;
    }

    function donateToken(address, address, uint256) external {
        unused += 1;
    }

    function approveContract(address, uint256) external {
        unused += 1;
    }

    function approveTarget(address, uint256) external {
        unused += 1;
    }

    function warpForward(uint256) external {
        unused += 1;
    }

    function transferFrom(address, address, uint256) external {
        unused += 1;
    }

    function report() external {
        unused += 1;
    }

    function setEmergencyAdmin(address) external {
        unused += 1;
    }

    function tend() external {
        unused += 1;
    }

    function emergencyWithdraw(uint256) external {
        unused += 1;
    }

    function setPerformanceFeeRecipient(address) external {
        unused += 1;
    }

    function setKeeper(address) external {
        unused += 1;
    }

    function mint(uint256, address) external {
        unused += 1;
    }

    function initialize(address, uint256) external {
        unused += 1;
    }

    function withdraw(uint256, address, address, uint256) external {
        unused += 1;
    }

    function transfer(address, uint256) external {
        unused += 1;
    }

    function setPerformanceFee(uint256) external {
        unused += 1;
    }

    function withdraw2(uint256, address, address, uint256) external {
        unused += 1;
    }

    function redeem2(uint256, address, address, uint256) external {
        unused += 1;
    }

    function shutdownStrategy() external {
        unused += 1;
    }

    function setName(string calldata) external {
        unused += 1;
    }

    function acceptManagement() external {
        unused += 1;
    }

    function permit(address, uint256, uint256) external {
        unused += 1;
    }

    function setProfitMaxUnlockTime(uint256) external {
        unused += 1;
    }

    function setPendingManagement(address) external {
        unused += 1;
    }

    function value() external view returns (uint256) {
        return profit;
    }
}
