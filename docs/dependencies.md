# Dependencies

Ripfuzz fetches dependency packages from tarball URLs and remaps their imports
during compilation, so contracts can import shared libraries without vendoring
them into the project.

## Fetching a dependency

```bash
ripfuzz fetch <name> <url>
```

For example, to fetch the ripfuzz standard library:

```bash
ripfuzz fetch ripfuzz https://github.com/pyk/ripfuzz-std/archive/main.tar.gz
```

The command:

1. Downloads the tar.gz archive from `url`, rejecting responses larger than 256
   MiB
2. Hashes the archive as a sha2-256 multihash (the `build.zig.zon` format),
   e.g. `0x1220abc...`
3. Extracts the archive into `.ripfuzz/dependencies/<name>`, stripping the
   single root directory GitHub archives pack their files in
4. Refuses to replace an existing dependency whose recorded hash differs
5. Records the dependency under `[dependencies]` in `ripfuzz.toml`

```toml
[dependencies]
ripfuzz = { url = "https://github.com/pyk/ripfuzz-std/archive/main.tar.gz", hash = "0x1220abc..." }
```

The `[dependencies]` section is edited in place with `toml_edit`, so comments
and formatting elsewhere in `ripfuzz.toml` are preserved. Re-running the
command refreshes the extracted sources and keeps the recorded hash intact
while the archive content stays the same.

## Import remapping

`ripfuzz test`, `ripfuzz max`, and `ripfuzz exec` remap every dependency name
to its extracted sources before compiling:

- `ripfuzz/std.sol` resolves to `.ripfuzz/dependencies/ripfuzz/src/std.sol`
  when the dependency ships a `src` directory
- Otherwise the import resolves against the dependency root

Precedence, highest first:

1. `remappings` from the `[solc]` section of `ripfuzz.toml`
2. Dependency remappings
3. `remappings.txt` entries

A dependency that is not fetched yet fails to compile with a missing import.
Run `ripfuzz fetch` again to restore it.
