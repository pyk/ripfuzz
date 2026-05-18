contract DeploymentOog {
    uint256[200] public data;
    constructor() {
        for (uint256 i = 0; i < 200; i++) {
            data[i] = i;
        }
    }
}
