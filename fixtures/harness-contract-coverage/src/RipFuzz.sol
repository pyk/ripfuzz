// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

contract RipFuzz {
    uint256 private constant _UINT256_MAX =
        115792089237316195423570985008687907853269984665640564039457584007913129639935;

    function _bound(
        uint256 x,
        uint256 min,
        uint256 max
    ) internal pure returns (uint256 result) {
        require(
            min <= max,
            "bound(uint256,uint256,uint256): Max is less than min."
        );
        if (x >= min && x <= max) return x;

        uint256 size = max - min + 1;

        if (x <= 3 && size > x) return min + x;
        if (x >= _UINT256_MAX - 3 && size > _UINT256_MAX - x)
            return max - (_UINT256_MAX - x);

        if (x > max) {
            uint256 diff = x - max;
            uint256 rem = diff % size;
            if (rem == 0) return max;
            result = min + rem - 1;
        } else if (x < min) {
            uint256 diff = min - x;
            uint256 rem = diff % size;
            if (rem == 0) return min;
            result = max - rem + 1;
        }
    }

    function bound(
        uint256 x,
        uint256 min,
        uint256 max
    ) internal pure returns (uint256 result) {
        result = _bound(x, min, max);
    }
}
