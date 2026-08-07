// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";

/// @notice Integration test fixture for verifying that the `rvm.store`
/// cheatcode correctly writes storage to local and remote contracts
/// in fork mode.
contract RvmStore {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    LocalStoreContract public localContract;

    /// Deploy a new LocalStoreContract.
    function setup() external {
        localContract = new LocalStoreContract();
    }

    /// Use rvm.store to write to storage slot 0 of the local contract
    /// and assert the value via the public getter.
    function storeLocalContract() external {
        rvm.store(address(localContract), bytes32(uint256(0)), bytes32(uint256(99)));
        require(localContract.value() == 99, "local store mismatch");
    }

    /// Use rvm.store to write to the WETH balanceOf slot for a locally
    /// derived address and assert the value via the balanceOf public
    /// getter.
    function storeRemoteContract() external {
        address local = rvm.addr(1);
        address weth = 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2;
        bytes32 balanceSlot = keccak256(abi.encode(local, uint256(3)));
        uint256 newBalance = 999;
        rvm.store(weth, balanceSlot, bytes32(newBalance));
        uint256 actualBalance = IWETH(weth).balanceOf(local);
        require(actualBalance == newBalance, "remote store mismatch");
    }
}

/// Simple contract with a public getter for slot 0.
contract LocalStoreContract {
    uint256 public value;
}

/// Minimal WETH balanceOf interface.
interface IWETH {
    function balanceOf(address) external view returns (uint256);
}
