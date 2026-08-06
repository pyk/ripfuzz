// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import "./Vm.sol";
import "./Counter.sol";
import "./AltCounter.sol";

contract GetCodeHarness {
    Vm constant vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);

    uint256 constant EXPECTED_VALUE = 42;

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

    /// Re-fetch Counter initcode, deploy and restore the canonical value.
    function actionGetCode() external {
        bytes memory code = vm.getCode("src/Counter.sol:Counter");
        address addr = deploy(code);
        deployedAddress = addr;
        storedValue = Counter(addr).getValue();
    }

    /// Fetch a different artifact to mutate stored state.
    function actionMutateGetCode() external {
        bytes memory code = vm.getCode("src/AltCounter.sol:AltCounter");
        address addr = deploy(code);
        deployedAddress = addr;
        storedValue = AltCounter(addr).getValue();
    }

    /// Interleave multiple vm.getCode calls with different artifacts,
    /// ending on the expected value to prove determinism.
    function actionGetCodeSequence() external returns (uint256 first, uint256 second, uint256 third) {
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

    function invariant_getCode() external view {
        assert(storedValue == EXPECTED_VALUE);
    }

    function deploy(bytes memory code) internal returns (address addr) {
        assembly {
            addr := create(0, add(code, 0x20), mload(code))
        }
        require(addr != address(0), "deployment failed");
    }
}
