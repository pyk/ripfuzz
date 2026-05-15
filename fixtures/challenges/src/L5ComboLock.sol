// SPDX-License-Identifier: UNLICENSED
pragma solidity 0.8.28;

/**
 * @title ComboLock
 * @notice Prime + Even + Odd in order -> 🐲
 * @dev Level 5: A complex multi-step combination.
 *      First call prime(17), then even(42), then odd(99).
 *      Any wrong value or wrong order resets everything.
 */
contract ComboLock {
    uint256 public property;
    uint256 internal _step; // 0 = idle, 1 = prime done, 2 = even done

    constructor() {
        property = 1 ether;
    }

    function prime(uint256 n) external {
        if (_step == 0 && _isPrime(n)) {
            _step = 1;
        } else {
            _step = 0;
        }
    }

    function even(uint256 n) external {
        if (_step == 1 && n % 2 == 0) {
            _step = 2;
        } else {
            _step = 0;
        }
    }

    function odd(uint256 n) external {
        if (_step == 2 && n % 2 == 1) {
            property = 5 ether;
        } else {
            _step = 0;
        }
    }

    /// @return true when the dragon is caught.
    function property_caught() external view returns (bool) {
        return property == 5 ether;
    }

    function _isPrime(uint256 n) internal pure returns (bool) {
        if (n < 2) return false;
        if (n == 2) return true;
        if (n % 2 == 0) return false;
        for (uint256 i = 3; i * i <= n; i += 2) {
            if (n % i == 0) return false;
        }
        return true;
    }
}
