// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

import {Market} from "./Market.sol";

contract MarketFactory {
    function createMarket() external returns (Market) {
        return new Market();
    }
}
