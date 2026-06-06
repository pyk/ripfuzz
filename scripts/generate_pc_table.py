#!/usr/bin/env python3
"""Generate a PC-to-source-map table from Foundry build artifacts.

Usage:
    python3 generate_pc_table.py out/ src/Contract.sol:Contract
    python3 generate_pc_table.py out/ src/Contract.sol:Contract --bytecode
    python3 generate_pc_table.py out/ src/Contract.sol:Contract --deployed-bytecode
    python3 generate_pc_table.py out/ src/Contract.sol:Contract --filter 6,13
    python3 generate_pc_table.py out/ src/Contract.sol:Contract > table.md
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path


def parse_source_map(source_map_str: str) -> list[dict]:
    """Parse a Solidity source map string with field reuse semantics."""
    entries: list[dict] = []
    current: dict[str, object] = {'s': None, 'l': None, 'f': None, 'j': None, 'm': None}
    for seg in source_map_str.split(';'):
        if seg:
            parts = seg.split(':')
            if len(parts) > 0 and parts[0]:
                current['s'] = int(parts[0])
            if len(parts) > 1 and parts[1]:
                current['l'] = int(parts[1])
            if len(parts) > 2 and parts[2]:
                current['f'] = int(parts[2])
            if len(parts) > 3 and parts[3]:
                current['j'] = parts[3]
            if len(parts) > 4 and parts[4]:
                current['m'] = int(parts[4])
        entries.append(dict(current))
    return entries


def load_artifact(path: Path) -> dict:
    """Load a single artifact JSON file."""
    return json.loads(path.read_text())


def resolve_artifact(
    artifacts_dir: Path,
    artifact_id: str,
) -> Path:
    """Resolve 'src/File.sol:Contract' to an artifact JSON path.

    Tries multiple strategies because Foundry strips directory prefixes
    from the artifact output structure:

    1. ``out/src/File.sol/Contract.json`` (full path)
    2. ``out/File.sol/Contract.json``  (filename only)
    """
    parts = artifact_id.rsplit(':', 1)
    if len(parts) != 2:
        raise SystemExit(f"Invalid artifact-id '{artifact_id}' (expected path:name)")
    rel_path, name = parts
    candidates = [
        artifacts_dir / rel_path / f'{name}.json',
        artifacts_dir / Path(rel_path).name / f'{name}.json',
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    raise SystemExit(f"Artifact not found. Tried: {', '.join(str(c) for c in candidates)}")


def build_source_id_map(
    artifacts_dir: Path,
    artifact_data: dict,
    artifact_path: str,
) -> dict[int, str]:
    """Build file-ID -> path map from build-info files.

    Loads all build-info JSON files from ``out/build-info/``, most recent
    first, and returns the ``source_id_to_path`` map from the one that
    contains ``artifact_path`` with the same numeric source ID as the
    artifact's ``id`` field.

    This matches the logic in ``src/foundry/build_info.rs`` and ensures the
    returned map is consistent with the source IDs in the artifact's
    bytecode source map.
    """
    artifact_source_id = artifact_data.get('id')
    if artifact_source_id is None:
        raise SystemExit("Artifact is missing 'id' field")

    build_info_dir = artifacts_dir / 'build-info'
    if not build_info_dir.is_dir():
        raise SystemExit(
            f"build-info directory not found: {build_info_dir}. "
            "Run `forge build --ast --extra-output storageLayout` first."
        )

    # Collect JSON files sorted by modification time (most recent first).
    files: list[Path] = []
    for entry in build_info_dir.iterdir():
        if entry.suffix != '.json':
            continue
        if entry.is_file():
            files.append(entry)
    files.sort(key=lambda p: p.stat().st_mtime, reverse=True)

    key = str(artifact_source_id)
    for path in files:
        info = json.loads(path.read_text())
        sid_map = info.get('source_id_to_path', {})
        if sid_map.get(key) == artifact_path:
            # Found the matching build-info. Build and return its map.
            result: dict[int, str] = {}
            for k, v in sid_map.items():
                try:
                    result[int(k)] = v
                except ValueError:
                    continue
            return result

    raise SystemExit(
        f"No build-info file found for source path '{artifact_path}' "
        f"with source ID {artifact_source_id}. "
        "Run `forge clean && forge build --ast --extra-output storageLayout` first."
    )


def get_content(src: str, offset: int, length: int, max_len: int = 50) -> str:
    """Extract source text at a byte offset, truncated for display."""
    if not src or offset >= len(src):
        return ""
    end = min(offset + min(length, max_len), len(src))
    content = src[offset:end].replace('\n', '\\n').replace('|', '\\|')
    if end < offset + length:
        content += "..."
    return content


def _cell(value: str) -> str:
    """Wrap a table cell in backticks, or leave empty if value is blank."""
    return f"`{value}`" if value else ""


def generate_table(
    artifacts_dir: Path,
    artifact_data: dict,
    artifact_id: str,
    filter_ids: set[int] | None = None,
    use_deployed: bool = True,
) -> str:
    """Generate a markdown table from a Foundry artifact and its project.

    Args:
        artifacts_dir: Path to Foundry's ``out/`` directory.
        artifact_data: Parsed artifact JSON.
        artifact_id: The artifact identifier string (e.g. ``src/File.sol:Contract``).
        filter_ids: If set, only emit rows for these source map file IDs.
        use_deployed: Use ``deployedBytecode`` if True, else ``bytecode``
            (initcode / constructor).
    """
    key = 'deployedBytecode' if use_deployed else 'bytecode'
    section = artifact_data.get(key, {})
    bc = section.get('object', '')
    sm = section.get('sourceMap', '')
    if not bc or not sm:
        label = 'deployedBytecode' if use_deployed else 'bytecode'
        return f"Error: artifact missing {label}.object or sourceMap."

    parsed = parse_source_map(sm)

    result = subprocess.run(
        ['cast', 'disassemble'],
        capture_output=True, text=True, input=bc,
    )
    if result.returncode != 0:
        return f"Error: cast disassemble failed:\n{result.stderr}"
    lines = result.stdout.strip().split('\n')

    source_path = artifact_id.rsplit(':', 1)[0]
    id_map = build_source_id_map(artifacts_dir, artifact_data, source_path)

    # Read resolved source files.
    source_cache: dict[int, str] = {}
    for fid, path in id_map.items():
        full = artifacts_dir.parent / path
        try:
            source_cache[fid] = full.read_text()
        except (OSError, UnicodeDecodeError):
            source_cache[fid] = ""

    rows: list[str] = []
    for i, line in enumerate(lines):
        entry = parsed[i] if i < len(parsed) else {}
        parts = line.split(': ', 1)
        if len(parts) != 2:
            continue
        pc = parts[0].strip()
        ins = parts[1].strip()
        # Truncate PUSH data to 4 bytes for readability.
        if ins.startswith('PUSH') and len(ins) > 13:
            # Format: PUSH<N> 0x<hex>  ->  PUSH<N> 0x<first 8 chars>...
            base = ins.split(' ')[0]  # e.g. PUSH32
            data_part = ins[len(base) + 1:]  # e.g. 0x4e487b710000...
            if data_part.startswith('0x') and len(data_part) > 10:
                ins = f"{base} {data_part[:10]}..."
            elif len(ins) > 35:
                ins = ins[:32] + '...'
        elif len(ins) > 35:
            ins = ins[:32] + '...'

        s = entry.get('s')
        l = entry.get('l')
        f = entry.get('f')
        j = entry.get('j', '-')
        m = entry.get('m', 0)

        if filter_ids is not None and (f is None or f not in filter_ids):
            continue

        s_str = str(s) if s is not None else ''
        l_str = str(l) if l is not None else ''
        f_str = str(f) if f is not None else ''

        sm_parts = [s_str, l_str, f_str]
        if j and j != '-':
            sm_parts.append(j)
        if m and m != 0:
            sm_parts.append(str(m) if m is not None else '')
        sm_display = ':'.join(sm_parts).removesuffix('::')

        fid_str = str(f) if f is not None else ''
        file_path = id_map.get(f, '') if f is not None else ''
        source_range = f"{s}:{l}" if s is not None and l is not None else ''

        content = ''
        if f is not None and s is not None and l is not None:
            src = source_cache.get(f, '')
            content = get_content(src, s, l)

        rows.append(
            f"| `{pc}` | `{ins}` | {_cell(sm_display)} | {_cell(fid_str)} | "
            f"{_cell(file_path)} | {_cell(source_range)} | {_cell(content)} |"
        )

    header = "| PC | Instruction | Source Map | File ID | File Path | Range | Content |\n"
    sep =    "| -- | ----------- | ---------- | ------- | --------- | ----- | ------- |\n"
    return header + sep + '\n'.join(rows) + '\n'


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate a PC-to-source-map table from a Foundry artifact.",
    )
    parser.add_argument(
        "artifacts_dir",
        type=Path,
        help="Path to Foundry's out/ directory.",
    )
    parser.add_argument(
        "artifact_id",
        help="Artifact identifier (e.g. src/Contract.sol:Contract).",
    )
    group = parser.add_mutually_exclusive_group()
    group.add_argument(
        "--deployed-bytecode",
        action="store_true",
        default=True,
        help="Use deployedBytecode (default).",
    )
    group.add_argument(
        "--bytecode",
        action="store_true",
        help="Use bytecode (initcode / constructor).",
    )
    parser.add_argument(
        "--filter",
        help="Comma-separated file IDs to include (e.g. 6,13).",
    )

    args = parser.parse_args()

    if not args.artifacts_dir.is_dir():
        raise SystemExit(f"Not a directory: {args.artifacts_dir}")

    use_deployed = not args.bytecode

    filter_ids: set[int] | None = None
    if args.filter:
        filter_ids = {int(x) for x in args.filter.split(',')}

    artifact_path = resolve_artifact(args.artifacts_dir, args.artifact_id)
    artifact_data = load_artifact(artifact_path)

    print(generate_table(args.artifacts_dir, artifact_data, args.artifact_id, filter_ids, use_deployed))


if __name__ == '__main__':
    main()
