// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./Counter.sol";
import "./AltCounter.sol";

contract GetCodeTarget {
    Vm constant vm = Vm(0x263Af513A0435EBC9D5C362Cf76252F87173F8f1);

    uint256 public storedValue;
    address public deployedAddress;

    function setup() external {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
        address addr = deploy(code);
        deployedAddress = addr;
        storedValue = Counter(addr).getValue();
    }

    function getDeployedValue() external view returns (uint256) {
        return Counter(deployedAddress).getValue();
    }

    function getStoredValue() external view returns (uint256) {
        return storedValue;
    }

    /// Call vm.getCode with the same artifact twice in one tx to prove
    /// the cheatcode is deterministic.
    function callGetCodeSameValueTwice()
        external
        returns (uint256 first, uint256 second)
    {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
        address addr1 = deploy(code);
        first = Counter(addr1).getValue();
        address addr2 = deploy(code);
        second = Counter(addr2).getValue();
    }

    /// Call vm.getCode with different artifacts and interleave to prove
    /// sequence independence and code uniqueness.
    function callGetCodeSequence()
        external
        returns (uint256 first, uint256 second, uint256 third)
    {
        bytes memory code1 = vm.getCode("src/Counter.sol:Counter");
        address addr1 = deploy(code1);
        first = Counter(addr1).getValue();

        bytes memory code2 = vm.getCode("src/AltCounter.sol:AltCounter");
        address addr2 = deploy(code2);
        second = AltCounter(addr2).getValue();

        bytes memory code3 = vm.getCode("src/Counter.sol:Counter");
        address addr3 = deploy(code3);
        third = Counter(addr3).getValue();
    }

    /// Interaction with warp - both cheatcodes in same tx.
    function callGetCodeAndWarp()
        external
        returns (uint256 value, uint256 timestamp)
    {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
        address addr = deploy(code);
        value = Counter(addr).getValue();
        vm.warp(1234567890);
        timestamp = block.timestamp;
    }

    /// Fuzzing action: re-fetch code, deploy and store the result.
    function actionGetCode() external {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
        address addr = deploy(code);
        deployedAddress = addr;
        storedValue = Counter(addr).getValue();
    }

    function invariant_get_code() external view {
        assert(storedValue == 42);
    }

    function deploy(bytes memory code) internal returns (address addr) {
        assembly {
            addr := create(0, add(code, 0x20), mload(code))
        }
        require(addr != address(0), "deployment failed");
    }
}
