// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

import "./Base.sol";

/// @title App fixture
/// @notice Fixture contract exercising every symbol kind of the inspector.
contract App is Base {
    /// @notice Thrown when the amount is zero.
    error Empty();

    /// @notice Emitted when the value changes.
    event ValueChanged(uint256 indexed newValue);

    /// @notice Kinds of values.
    enum Kind {
        First,
        Second
    }

    /// @notice A point in space.
    struct Point {
        uint256 x;
        uint256 y;
    }

    /// @notice The scaling factor.
    uint256 internal constant PRECISION = 1e18;

    /// @notice The owner gate.
    address public owner;

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    function configure(address newOwner) external payable onlyOwner {
        owner = newOwner;
        emit ValueChanged(total);
    }

    function read() external view returns (uint256) {
        return total;
    }

    function mid(Point memory point) external view returns (uint256) {
        return _mid(point);
    }

    function _mid(Point memory point) internal view returns (uint256) {
        uint256 sum = point.x + point.y;
        if (sum == 0) {
            revert Empty();
        }
        return (sum * PRECISION) / 2;
    }

    function kind(uint256 flag) external pure returns (Kind) {
        if (flag > 1) {
            return Kind.Second;
        }
        return Kind.First;
    }
}
