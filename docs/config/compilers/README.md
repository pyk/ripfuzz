# Compiler Configuration

Ripfuzz compiles Solidity harnesses, scripts, and targets with `solc`. The
`[solc]` section of `ripfuzz.toml` configures the compiler.

| Document           | Contents                                            |
| :----------------- | :-------------------------------------------------- |
| [solc.md](solc.md) | The `[solc]` section: version, output, and settings |

The minimal config requires only the solc version. Ripfuzz never detects the
version automatically:

```toml
[solc]
version = "0.8.36"
```

See [solc.md](solc.md) for every option, its default, and worked examples.
