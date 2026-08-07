// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {RVM} from "../src/RVM.sol";
import {MarketFactory} from "./MarketFactory.sol";
import {Market} from "./Market.sol";

/// @notice Regression fixture for a bug where deploying a child contract
/// inside a handler function in fork mode caused an unnecessary RPC fetch
/// for the child's address.
contract DeployChildInHandlerFunction {
    RVM constant rvm = RVM(0x628dC59F11F72B611132eC40437F125ba1312F08);

    MarketFactory public factory;
    Market public market;

    function setup() external {
        rvm.fork("mock://test", 25_259_523);
        factory = new MarketFactory();
    }

    function createMarket() external {
        market = factory.createMarket();
    }

    function checkMarket() external view {
        require(market.value() == 0, "market value must be zero");
    }

    function invariant_market_exists() external view {
        assert(address(market) != address(0));
    }
}
