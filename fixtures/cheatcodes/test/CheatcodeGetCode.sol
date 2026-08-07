// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

import {RVM} from "../src/RVM.sol";
import {Helper} from "../src/Helper.sol";

contract CheatcodeGetCode {
    RVM constant rvm = RVM(address(0x628dC59F11F72B611132eC40437F125ba1312F08));

    address public setupAddr;
    address public actionAddr;
    address public actionAddr2;
    bytes public selfCode;
    bytes public bareCode;
    bytes public fileCode;
    bytes public fullCode;
    bool public missingReverted;

    function setup() external {
        bytes memory code = rvm.getCode("Helper");
        assembly {
            sstore(setupAddr.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_deployHelper() external {
        bytes memory code = rvm.getCode("Helper");
        assembly {
            sstore(actionAddr.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_deployHelperAgain() external {
        bytes memory code = rvm.getCode("Helper");
        assembly {
            sstore(actionAddr2.slot, create(0, add(code, 0x20), mload(code)))
        }
    }

    function action_getMissingCode() external {
        try rvm.getCode("NonExistentContract") {
            missingReverted = false;
        } catch {
            missingReverted = true;
        }
    }

    function action_getCodeBare() external {
        bareCode = rvm.getCode("Helper");
    }

    function action_getCodeFile() external {
        fileCode = rvm.getCode("Helper.sol");
    }

    function action_getCodeFull() external {
        fullCode = rvm.getCode("Helper.sol:Helper");
    }

    function action_getSelfCode() external {
        selfCode = rvm.getCode("CheatcodeGetCode");
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
