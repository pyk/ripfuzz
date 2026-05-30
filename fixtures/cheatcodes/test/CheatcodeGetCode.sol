// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {Vm} from "../src/Vm.sol";
import {Helper} from "../src/Helper.sol";

contract CheatcodeGetCode {
    Vm constant vm = Vm(address(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D));

    address public setupAddr;
    address public actionAddr;
    address public actionAddr2;
    bytes public selfCode;
    bytes public bareCode;
    bytes public fileCode;
    bytes public fullCode;
    bool public missingReverted;

    function setup() external {
        bytes memory code = vm.getCode("Helper");
        assembly {
            sstore(setupAddr.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_deployHelper() external {
        bytes memory code = vm.getCode("Helper");
        assembly {
            sstore(actionAddr.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_deployHelperAgain() external {
        bytes memory code = vm.getCode("Helper");
        assembly {
            sstore(actionAddr2.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_getMissingCode() external {
        try vm.getCode("NonExistentContract") {
            missingReverted = false;
        } catch {
            missingReverted = true;
        }
    }

    function action_getCodeBare() external {
        bareCode = vm.getCode("Helper");
    }

    function action_getCodeFile() external {
        fileCode = vm.getCode("Helper.sol");
    }

    function action_getCodeFull() external {
        fullCode = vm.getCode("Helper.sol:Helper");
    }

    function action_getSelfCode() external {
        selfCode = vm.getCode("CheatcodeGetCode");
    }

    function setupGetCode() external view returns (bool) {
        return setupAddr != address(0) && Helper(setupAddr).magic() == 42;
    }

    function actionGetCode() external view returns (bool) {
        return actionAddr != address(0) && Helper(actionAddr).magic() == 42;
    }

    function multiGetCode() external view returns (bool) {
        return actionAddr != address(0)
            && actionAddr2 != address(0)
            && actionAddr != actionAddr2
            && Helper(actionAddr).magic() == 42
            && Helper(actionAddr2).magic() == 42;
    }

    function errorCase() external view returns (bool) {
        return missingReverted;
    }

    function formats() external view returns (bool) {
        return keccak256(bareCode) == keccak256(fileCode)
            && keccak256(fileCode) == keccak256(fullCode);
    }

    function selfLookup() external view returns (bool) {
        return selfCode.length > 0;
    }
}
