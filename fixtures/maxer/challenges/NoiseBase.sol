// SPDX-License-Identifier: MIT
pragma solidity 0.8.36;

/// Base contract that adds noise for max challenges.
///
/// Half of the functions revert unconditionally, and the other half mutate
/// state that no harness `value` function reads.
abstract contract NoiseBase {
    uint256 internal unused;

    function noiseRevert0() external pure {
        revert();
    }

    function noiseRevert1() external pure {
        revert();
    }

    function noiseRevert2() external pure {
        revert();
    }

    function noiseRevert3() external pure {
        revert();
    }

    function noiseRevert4() external pure {
        revert();
    }

    function noiseRevert5() external pure {
        revert();
    }

    function noiseRevert6() external pure {
        revert();
    }

    function noiseRevert7() external pure {
        revert();
    }

    function noiseRevert8() external pure {
        revert();
    }

    function noiseRevert9() external pure {
        revert();
    }

    function noiseWrite0() external {
        unused += 1;
    }

    function noiseWrite1() external {
        unused -= 1;
    }

    function noiseWrite2() external {
        unused *= 3;
    }

    function noiseWrite3() external {
        unused = ~unused;
    }

    function noiseWrite4() external {
        unused <<= 1;
    }

    function noiseWrite5() external {
        unused >>= 1;
    }

    function noiseWrite6() external {
        unused ^= 0xff;
    }

    function noiseWrite7() external {
        unused |= 0xff;
    }

    function noiseWrite8() external {
        unused &= ~uint256(0xff);
    }

    function noiseWrite9() external {
        unused = 0;
    }
}
