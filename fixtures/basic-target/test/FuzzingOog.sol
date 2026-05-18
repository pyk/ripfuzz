contract FuzzingOog {
    uint256[200] public data;

    function gasHog() public {
        for (uint256 i = 0; i < 200; i++) {
            data[i] = i;
        }
    }

    function invariant_always_true() public pure returns (bool) {
        return true;
    }
}
