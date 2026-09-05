// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Base.sol";

contract App is Base {
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    function configure(address newOwner) external payable onlyOwner {
        owner = newOwner;
    }

    function read() external view returns (uint256) {
        return total;
    }

    function onERC721Received(address, address, uint256, bytes calldata) external pure returns (bytes4) {
        return this.onERC721Received.selector;
    }

    receive() external payable {}

    fallback() external {}
}
