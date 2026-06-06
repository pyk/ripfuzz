| PC         | Instruction            | Source Map  | File ID | File Path                     | Range     | Content                                                  |
| ---------- | ---------------------- | ----------- | ------- | ----------------------------- | --------- | -------------------------------------------------------- |
| `00000000` | `PUSH1 0x80`           | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `00000002` | `PUSH1 0x40`           | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `00000004` | `MSTORE`               | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `00000005` | `CALLVALUE`            | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `00000006` | `DUP1`                 | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `00000007` | `ISZERO`               | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `00000008` | `PUSH1 0x0e`           | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000a` | `JUMPI`                | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000b` | `PUSH0`                | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000c` | `PUSH0`                | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000d` | `REVERT`               | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000e` | `JUMPDEST`             | `186:46:6`  | `6`     | `src/TargetContractBasic.sol` | `186:46`  | `constructor() {\n        latestValue = 0;\n    }`       |
| `0000000f` | `POP`                  | `-1:-1:-1`  | `-1`    | `src/RaptorFuzz.sol`          | `-1:-1`   |                                                          |
| `00000010` | `PUSH0`                | `224:1:6`   | `6`     | `src/TargetContractBasic.sol` | `224:1`   | `0`                                                      |
| `00000011` | `DUP1`                 | `210:15:6`  | `6`     | `src/TargetContractBasic.sol` | `210:15`  | `latestValue = 0`                                        |
| `00000012` | `SSTORE`               | `210:15:6`  | `6`     | `src/TargetContractBasic.sol` | `210:15`  | `latestValue = 0`                                        |
| `00000013` | `PUSH2 0x0151`         | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `00000016` | `DUP1`                 | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `00000017` | `PUSH2 0x001f`         | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `0000001a` | `PUSH0`                | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `0000001b` | `CODECOPY`             | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `0000001c` | `PUSH0`                | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `0000001d` | `RETURN`               | `104:730:6` | `6`     | `src/TargetContractBasic.sol` | `104:730` | `contract TargetContractBasic is RaptorFuzz {\n    u...` |
| `0000001e` | `INVALID`              |             |         |                               |           |                                                          |
| `0000001f` | `PUSH1 0x80`           |             |         |                               |           |                                                          |
| `00000021` | `PUSH1 0x40`           |             |         |                               |           |                                                          |
| `00000023` | `MSTORE`               |             |         |                               |           |                                                          |
| `00000024` | `CALLVALUE`            |             |         |                               |           |                                                          |
| `00000025` | `DUP1`                 |             |         |                               |           |                                                          |
| `00000026` | `ISZERO`               |             |         |                               |           |                                                          |
| `00000027` | `PUSH2 0x000f`         |             |         |                               |           |                                                          |
| `0000002a` | `JUMPI`                |             |         |                               |           |                                                          |
| `0000002b` | `PUSH0`                |             |         |                               |           |                                                          |
| `0000002c` | `PUSH0`                |             |         |                               |           |                                                          |
| `0000002d` | `REVERT`               |             |         |                               |           |                                                          |
| `0000002e` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `0000002f` | `POP`                  |             |         |                               |           |                                                          |
| `00000030` | `PUSH1 0x04`           |             |         |                               |           |                                                          |
| `00000032` | `CALLDATASIZE`         |             |         |                               |           |                                                          |
| `00000033` | `LT`                   |             |         |                               |           |                                                          |
| `00000034` | `PUSH2 0x0034`         |             |         |                               |           |                                                          |
| `00000037` | `JUMPI`                |             |         |                               |           |                                                          |
| `00000038` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000039` | `CALLDATALOAD`         |             |         |                               |           |                                                          |
| `0000003a` | `PUSH1 0xe0`           |             |         |                               |           |                                                          |
| `0000003c` | `SHR`                  |             |         |                               |           |                                                          |
| `0000003d` | `DUP1`                 |             |         |                               |           |                                                          |
| `0000003e` | `PUSH4 0x19e9d629`     |             |         |                               |           |                                                          |
| `00000043` | `EQ`                   |             |         |                               |           |                                                          |
| `00000044` | `PUSH2 0x0038`         |             |         |                               |           |                                                          |
| `00000047` | `JUMPI`                |             |         |                               |           |                                                          |
| `00000048` | `DUP1`                 |             |         |                               |           |                                                          |
| `00000049` | `PUSH4 0x4ab5cc82`     |             |         |                               |           |                                                          |
| `0000004e` | `EQ`                   |             |         |                               |           |                                                          |
| `0000004f` | `PUSH2 0x005d`         |             |         |                               |           |                                                          |
| `00000052` | `JUMPI`                |             |         |                               |           |                                                          |
| `00000053` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000054` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000055` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000056` | `REVERT`               |             |         |                               |           |                                                          |
| `00000057` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000058` | `PUSH2 0x004b`         |             |         |                               |           |                                                          |
| `0000005b` | `PUSH2 0x0046`         |             |         |                               |           |                                                          |
| `0000005e` | `CALLDATASIZE`         |             |         |                               |           |                                                          |
| `0000005f` | `PUSH1 0x04`           |             |         |                               |           |                                                          |
| `00000061` | `PUSH2 0x00a8`         |             |         |                               |           |                                                          |
| `00000064` | `JUMP`                 |             |         |                               |           |                                                          |
| `00000065` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000066` | `PUSH2 0x0065`         |             |         |                               |           |                                                          |
| `00000069` | `JUMP`                 |             |         |                               |           |                                                          |
| `0000006a` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `0000006b` | `PUSH1 0x40`           |             |         |                               |           |                                                          |
| `0000006d` | `MLOAD`                |             |         |                               |           |                                                          |
| `0000006e` | `SWAP1`                |             |         |                               |           |                                                          |
| `0000006f` | `DUP2`                 |             |         |                               |           |                                                          |
| `00000070` | `MSTORE`               |             |         |                               |           |                                                          |
| `00000071` | `PUSH1 0x20`           |             |         |                               |           |                                                          |
| `00000073` | `ADD`                  |             |         |                               |           |                                                          |
| `00000074` | `PUSH1 0x40`           |             |         |                               |           |                                                          |
| `00000076` | `MLOAD`                |             |         |                               |           |                                                          |
| `00000077` | `DUP1`                 |             |         |                               |           |                                                          |
| `00000078` | `SWAP2`                |             |         |                               |           |                                                          |
| `00000079` | `SUB`                  |             |         |                               |           |                                                          |
| `0000007a` | `SWAP1`                |             |         |                               |           |                                                          |
| `0000007b` | `RETURN`               |             |         |                               |           |                                                          |
| `0000007c` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `0000007d` | `PUSH2 0x004b`         |             |         |                               |           |                                                          |
| `00000080` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000081` | `SLOAD`                |             |         |                               |           |                                                          |
| `00000082` | `DUP2`                 |             |         |                               |           |                                                          |
| `00000083` | `JUMP`                 |             |         |                               |           |                                                          |
| `00000084` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000085` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000086` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000087` | `PUSH2 0x0071`         |             |         |                               |           |                                                          |
| `0000008a` | `DUP5`                 |             |         |                               |           |                                                          |
| `0000008b` | `DUP5`                 |             |         |                               |           |                                                          |
| `0000008c` | `PUSH2 0x0087`         |             |         |                               |           |                                                          |
| `0000008f` | `JUMP`                 |             |         |                               |           |                                                          |
| `00000090` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000091` | `SWAP1`                |             |         |                               |           |                                                          |
| `00000092` | `POP`                  |             |         |                               |           |                                                          |
| `00000093` | `PUSH2 0x007d`         |             |         |                               |           |                                                          |
| `00000096` | `DUP2`                 |             |         |                               |           |                                                          |
| `00000097` | `DUP5`                 |             |         |                               |           |                                                          |
| `00000098` | `PUSH2 0x009d`         |             |         |                               |           |                                                          |
| `0000009b` | `JUMP`                 |             |         |                               |           |                                                          |
| `0000009c` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `0000009d` | `SWAP2`                |             |         |                               |           |                                                          |
| `0000009e` | `POP`                  |             |         |                               |           |                                                          |
| `0000009f` | `POP`                  |             |         |                               |           |                                                          |
| `000000a0` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000a1` | `SWAP3`                |             |         |                               |           |                                                          |
| `000000a2` | `SWAP2`                |             |         |                               |           |                                                          |
| `000000a3` | `POP`                  |             |         |                               |           |                                                          |
| `000000a4` | `POP`                  |             |         |                               |           |                                                          |
| `000000a5` | `JUMP`                 |             |         |                               |           |                                                          |
| `000000a6` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000a7` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000a8` | `PUSH2 0x0092`         |             |         |                               |           |                                                          |
| `000000ab` | `DUP3`                 |             |         |                               |           |                                                          |
| `000000ac` | `DUP5`                 |             |         |                               |           |                                                          |
| `000000ad` | `PUSH2 0x00f5`         |             |         |                               |           |                                                          |
| `000000b0` | `JUMP`                 |             |         |                               |           |                                                          |
| `000000b1` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000b2` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000b3` | `DUP2`                 |             |         |                               |           |                                                          |
| `000000b4` | `SWAP1`                |             |         |                               |           |                                                          |
| `000000b5` | `SSTORE`               |             |         |                               |           |                                                          |
| `000000b6` | `SWAP4`                |             |         |                               |           |                                                          |
| `000000b7` | `SWAP3`                |             |         |                               |           |                                                          |
| `000000b8` | `POP`                  |             |         |                               |           |                                                          |
| `000000b9` | `POP`                  |             |         |                               |           |                                                          |
| `000000ba` | `POP`                  |             |         |                               |           |                                                          |
| `000000bb` | `JUMP`                 |             |         |                               |           |                                                          |
| `000000bc` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000bd` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000be` | `PUSH2 0x0092`         |             |         |                               |           |                                                          |
| `000000c1` | `DUP3`                 |             |         |                               |           |                                                          |
| `000000c2` | `DUP5`                 |             |         |                               |           |                                                          |
| `000000c3` | `PUSH2 0x0108`         |             |         |                               |           |                                                          |
| `000000c6` | `JUMP`                 |             |         |                               |           |                                                          |
| `000000c7` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000c8` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000c9` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000ca` | `PUSH1 0x40`           |             |         |                               |           |                                                          |
| `000000cc` | `DUP4`                 |             |         |                               |           |                                                          |
| `000000cd` | `DUP6`                 |             |         |                               |           |                                                          |
| `000000ce` | `SUB`                  |             |         |                               |           |                                                          |
| `000000cf` | `SLT`                  |             |         |                               |           |                                                          |
| `000000d0` | `ISZERO`               |             |         |                               |           |                                                          |
| `000000d1` | `PUSH2 0x00b9`         |             |         |                               |           |                                                          |
| `000000d4` | `JUMPI`                |             |         |                               |           |                                                          |
| `000000d5` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000d6` | `PUSH0`                |             |         |                               |           |                                                          |
| `000000d7` | `REVERT`               |             |         |                               |           |                                                          |
| `000000d8` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000d9` | `POP`                  |             |         |                               |           |                                                          |
| `000000da` | `POP`                  |             |         |                               |           |                                                          |
| `000000db` | `DUP1`                 |             |         |                               |           |                                                          |
| `000000dc` | `CALLDATALOAD`         |             |         |                               |           |                                                          |
| `000000dd` | `SWAP3`                |             |         |                               |           |                                                          |
| `000000de` | `PUSH1 0x20`           |             |         |                               |           |                                                          |
| `000000e0` | `SWAP1`                |             |         |                               |           |                                                          |
| `000000e1` | `SWAP2`                |             |         |                               |           |                                                          |
| `000000e2` | `ADD`                  |             |         |                               |           |                                                          |
| `000000e3` | `CALLDATALOAD`         |             |         |                               |           |                                                          |
| `000000e4` | `SWAP2`                |             |         |                               |           |                                                          |
| `000000e5` | `POP`                  |             |         |                               |           |                                                          |
| `000000e6` | `JUMP`                 |             |         |                               |           |                                                          |
| `000000e7` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `000000e8` | `PUSH32 0x4e487b71...` |             |         |                               |           |                                                          |
| `00000109` | `PUSH0`                |             |         |                               |           |                                                          |
| `0000010a` | `MSTORE`               |             |         |                               |           |                                                          |
| `0000010b` | `PUSH1 0x11`           |             |         |                               |           |                                                          |
| `0000010d` | `PUSH1 0x04`           |             |         |                               |           |                                                          |
| `0000010f` | `MSTORE`               |             |         |                               |           |                                                          |
| `00000110` | `PUSH1 0x24`           |             |         |                               |           |                                                          |
| `00000112` | `PUSH0`                |             |         |                               |           |                                                          |
| `00000113` | `REVERT`               |             |         |                               |           |                                                          |
| `00000114` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000115` | `DUP1`                 |             |         |                               |           |                                                          |
| `00000116` | `DUP3`                 |             |         |                               |           |                                                          |
| `00000117` | `ADD`                  |             |         |                               |           |                                                          |
| `00000118` | `DUP1`                 |             |         |                               |           |                                                          |
| `00000119` | `DUP3`                 |             |         |                               |           |                                                          |
| `0000011a` | `GT`                   |             |         |                               |           |                                                          |
| `0000011b` | `ISZERO`               |             |         |                               |           |                                                          |
| `0000011c` | `PUSH2 0x0081`         |             |         |                               |           |                                                          |
| `0000011f` | `JUMPI`                |             |         |                               |           |                                                          |
| `00000120` | `PUSH2 0x0081`         |             |         |                               |           |                                                          |
| `00000123` | `PUSH2 0x00c8`         |             |         |                               |           |                                                          |
| `00000126` | `JUMP`                 |             |         |                               |           |                                                          |
| `00000127` | `JUMPDEST`             |             |         |                               |           |                                                          |
| `00000128` | `DUP2`                 |             |         |                               |           |                                                          |
| `00000129` | `DUP2`                 |             |         |                               |           |                                                          |
| `0000012a` | `SUB`                  |             |         |                               |           |                                                          |
| `0000012b` | `DUP2`                 |             |         |                               |           |                                                          |
| `0000012c` | `DUP2`                 |             |         |                               |           |                                                          |
| `0000012d` | `GT`                   |             |         |                               |           |                                                          |
| `0000012e` | `ISZERO`               |             |         |                               |           |                                                          |
| `0000012f` | `PUSH2 0x0081`         |             |         |                               |           |                                                          |
| `00000132` | `JUMPI`                |             |         |                               |           |                                                          |
| `00000133` | `PUSH2 0x0081`         |             |         |                               |           |                                                          |
| `00000136` | `PUSH2 0x00c8`         |             |         |                               |           |                                                          |
| `00000139` | `JUMP`                 |             |         |                               |           |                                                          |
| `0000013a` | `INVALID`              |             |         |                               |           |                                                          |
| `0000013b` | `LOG2`                 |             |         |                               |           |                                                          |
| `0000013c` | `PUSH5 0x69706673...`  |             |         |                               |           |                                                          |
| `00000142` | `UNKNOWN(0x22)`        |             |         |                               |           |                                                          |
| `00000143` | `SLT`                  |             |         |                               |           |                                                          |
| `00000144` | `KECCAK256`            |             |         |                               |           |                                                          |
| `00000145` | `UNKNOWN(0xEA)`        |             |         |                               |           |                                                          |
| `00000146` | `CREATE2`              |             |         |                               |           |                                                          |
| `00000147` | `PC`                   |             |         |                               |           |                                                          |
| `00000148` | `PUSH13 0x21f09bc3...` |             |         |                               |           |                                                          |
| `00000156` | `RETURN`               |             |         |                               |           |                                                          |
| `00000157` | `UNKNOWN(0xCF)`        |             |         |                               |           |                                                          |
| `00000158` | `UNKNOWN(0xB4)`        |             |         |                               |           |                                                          |
| `00000159` | `PUSH13 0x8df9cbda...` |             |         |                               |           |                                                          |
| `00000167` | `PUSH16`               |             |         |                               |           |                                                          |
