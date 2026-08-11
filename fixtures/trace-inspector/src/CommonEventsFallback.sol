// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

/// Emits the common standard events (ERC20, ERC721, WETH9, Ownable) through
/// inline assembly without declaring them in the ABI. Trace decoding must
/// fall back to the common events instead of rendering raw `Log(0x...)`
/// lines, while the undeclared custom event stays raw.
contract CommonEventsFallback {
    function dummy() external {}

    constructor() {
        address from = address(0);
        address to = address(0xBEEF);
        address operator = address(0xCAFE);
        uint256 value = 1000;

        // keccak256("Transfer(address,address,uint256)")
        bytes32 transferTopic = 0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef;
        // keccak256("Approval(address,address,uint256)")
        bytes32 approvalTopic = 0x8c5be1e5ebec7d5bd14f71427d1e84f3dd0314c0f7b2291e5b200ac8c7c3b925;
        // keccak256("ApprovalForAll(address,address,bool)")
        bytes32 approvalForAllTopic = 0x17307eab39ab6107e8899845ad3d59bd9653f200f220920489ca2b5937696c31;
        // keccak256("Deposit(address,uint256)")
        bytes32 depositTopic = 0xe1fffcc4923d04b559f4d29a8bfc6cda04eb5b0d3c460751c2402c5c5cc9109c;
        // keccak256("Withdrawal(address,uint256)")
        bytes32 withdrawalTopic = 0x7fcf532c15f0a6db0bd6d0e038bea71d30d808c7d98cb3bf7268a95bf5081b65;
        // keccak256("OwnershipTransferred(address,address)")
        bytes32 ownershipTransferredTopic = 0x8be0079c531659141344cd1fd0a4f28419497f9722a3daafe3b4186f6b6457e0;
        // keccak256("MysteryEvent(uint256)"): matches no known event.
        bytes32 unknownTopic = 0xa3e2302ebab281ec78f3a54c84cbdfa28353759406c94f52e65a0c59d63ef5f4;

        assembly {
            mstore(0, value)
            log3(0, 32, transferTopic, from, to)
            log3(0, 32, approvalTopic, from, to)
            // approved = true
            mstore(0, 1)
            log3(0, 32, approvalForAllTopic, from, operator)
            // wad = 1 ether
            mstore(0, 1000000000000000000)
            log2(0, 32, depositTopic, operator)
            // wad = 0.5 ether
            mstore(0, 500000000000000000)
            log2(0, 32, withdrawalTopic, operator)
            log3(0, 0, ownershipTransferredTopic, from, operator)
            // unknown event with value as data
            mstore(0, value)
            log1(0, 32, unknownTopic)
        }
    }
}
