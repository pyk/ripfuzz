// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// A harness with no failing paths. Used to verify campaigns with no failed
/// assertions finish cleanly without entering the shrinker.
contract NoFail {
    function ping() external pure {}
}
