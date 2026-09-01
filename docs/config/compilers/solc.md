# Solc Configuration

The `[solc]` section of `ripfuzz.toml` configures how ripfuzz compiles Solidity
with `solc`. It applies to every command that takes a `.sol` input:
`ripfuzz test`, `ripfuzz max`, and `ripfuzz exec`.

## Minimal config

Ripfuzz does not detect the solc version automatically, so the only required
option is `version`:

```toml
[solc]
version = "0.8.36"
```

A `ripfuzz.toml` whose input is a `.sol` file must define `solc.version`.
Missing it, or using the legacy flat `solc = "0.8.36"` form, fails with a
config parse error.

## Reference

```toml
[solc]
version = "0.8.36"
out = ".ripfuzz/solc"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
via_ir = true
remappings = [
    "@openzeppelin/=lib/openzeppelin-contracts/",
    "@uniswap/=node_modules/@uniswap/",
]
```

| Option           | Default         | Description                                      |
| :--------------- | :-------------- | :----------------------------------------------- |
| `version`        | required        | Solc version to compile with                     |
| `out`            | `.ripfuzz/solc` | Output directory for compilation artifacts       |
| `evm_version`    | `prague`        | Target EVM version, affects which opcodes exist  |
| `optimizer`      | `false`         | Enable the optimizer                             |
| `optimizer_runs` | `200`           | Number of optimizer runs                         |
| `via_ir`         | `false`         | Compile through the IR-based pipeline            |
| `remappings`     | `[]`            | Import path mappings, as `prefix=target` entries |

Relative `out` paths resolve against the project root.

### `evm_version`

Sets the target EVM version for compilation. This affects which opcodes are
available. Common values:

| Value      | Notes                                  |
| :--------- | :------------------------------------- |
| `paris`    | Last version before the withdrawal era |
| `shanghai` | Adds push0                             |
| `cancun`   | Adds transient storage and mcopy       |
| `prague`   | Default                                |

### `optimizer` and `optimizer_runs`

`optimizer` enables the Solidity optimizer. `optimizer_runs` sets how many
times the optimizer is allowed to run; more runs produce denser bytecode at the
cost of slower compilation. `optimizer_runs` only matters when `optimizer` is
`true`, but it always defaults to `200`.

### `via_ir`

`via_ir` compiles through the new IR-based code generation pipeline. It
produces better optimization at the cost of slower compilation, and is required
for some Yul-level features.

### `remappings`

Maps import paths to actual file locations. Each entry has the form
`prefix=target`, and both sides keep a trailing slash so prefixes only match
whole path segments.

Config remappings take precedence over remappings with the same prefix in
`{root}/remappings.txt`.

## Example: OpenZeppelin project

```toml
[solc]
version = "0.8.36"
evm_version = "cancun"
optimizer = true
optimizer_runs = 200
remappings = ["@openzeppelin/=lib/openzeppelin-contracts/"]
```

With this config, `import "@openzeppelin/contracts/token/ERC20.sol"` resolves
to `lib/openzeppelin-contracts/contracts/token/ERC20.sol` relative to the
project root.

## Artifacts

Compilation writes artifacts under `{out}/{target path}`, so targets sharing an
out directory never overwrite each other:

```text
.ripfuzz/solc/
  src/
    Harness.sol/
      out.json                     combined solc standard JSON output
      Harness.sol/Harness.json     per-contract artifact (abi, bytecode, ...)
```
