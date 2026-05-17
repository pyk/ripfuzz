// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

import {Vm} from "../src/Vm.sol";

contract CheatcodeState {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));
    uint256 public recordedTimestamp;
    uint256 public recordedNumber;
    uint256 public recordedFee;
    address public recordedCoinbase;
    bytes32 public recordedPrevrandao;
    uint256 public recordedChainId;

    function setUp() external {
        vm.warp(1234567890);
        vm.roll(42);
        vm.fee(10);
        vm.coinbase(address(0xCA11BA5E));
        vm.prevrandao(bytes32(uint256(0xDEADBEEF)));
        vm.chainId(1337);
    }

    function action() external {
        recordedTimestamp = block.timestamp;
        recordedNumber = block.number;
        recordedFee = block.basefee;
        recordedCoinbase = block.coinbase;
        recordedPrevrandao = bytes32(uint256(block.prevrandao));
        recordedChainId = block.chainid;
    }

    function property_state_correct() external view returns (bool) {
        return recordedTimestamp == 1234567890
            && recordedNumber == 42
            && recordedFee == 10
            && recordedCoinbase == address(0xCA11BA5E)
            && recordedPrevrandao == bytes32(uint256(0xDEADBEEF))
            && recordedChainId == 1337;
    }
}
