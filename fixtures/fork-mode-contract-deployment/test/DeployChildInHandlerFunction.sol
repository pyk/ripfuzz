// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {MarketFactory} from "./MarketFactory.sol";
import {Market} from "./Market.sol";

/// @notice Regression fixture for a bug where deploying a child contract
/// inside a handler function in fork mode caused an unnecessary RPC fetch
/// for the child's address.
contract DeployChildInHandlerFunction {
    MarketFactory public factory;
    Market public market;

    function setup() external {
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
