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


def find_artifacts(artifacts_dir: Path) -> list[Path]:
    """Find all artifact JSON files under out/ (skipping build-info)."""
    paths: list[Path] = []
    for root, dirs, files in os.walk(artifacts_dir):
        # Skip build-info directory.
        if 'build-info' in dirs:
            dirs.remove('build-info')
        for fname in files:
            if fname.endswith('.json'):
                paths.append(Path(root) / fname)
    return paths


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
) -> dict[int, str]:
    """Build file-ID -> path map by scanning all artifacts for source info.

    Mirrors raptor's SourceIdResolver: greedily match source IDs to candidate
    files by checking that all source map offsets fall within the file's byte
    length. Does not rely on build-info (which shifts during incremental
    builds).
    """
    id_map: dict[int, str] = {}

    # Step 1: the artifact's own ID.
    own_id = artifact_data.get('id')
    ast = artifact_data.get('ast', {})
    own_path = ast.get('absolutePath', '')
    if own_id is not None and own_path:
        id_map[own_id] = own_path

    # Step 2: collect candidate paths from this artifact's metadata.sources.
    metadata = artifact_data.get('metadata', {})
    sources = metadata.get('sources', {})
    candidates: set[str] = set(sources.keys())
    if own_path:
        candidates.add(own_path)

    # Step 3: collect *all* source IDs used in *all* artifacts' source maps.
    # Also track min and max offset per source ID. The max can be inflated by
    # compiler-generated code (ABI decoders etc.) attributed to the imported
    # file's ID. We use the minimum offset for file-size validation: if any
    # entry for a given source ID fits within a candidate file, the ID likely
    # belongs to that file.
    used_ids: set[int] = set()
    sid_min_offset: dict[int, int] = {}  # smallest s+l per source ID
    sid_max_offset: dict[int, int] = {}  # largest s+l per source ID

    all_artifacts = find_artifacts(artifacts_dir)
    for art_path in all_artifacts:
        try:
            art = load_artifact(art_path)
        except (json.JSONDecodeError, OSError):
            continue
        for sm_key in ['deployedBytecode', 'bytecode']:
            sm_str = art.get(sm_key, {}).get('sourceMap', '')
            for entry in parse_source_map(sm_str):
                fid = entry.get('f')
                if fid is None:
                    continue
                used_ids.add(fid)
                s_off = entry.get('s', 0) or 0
                s_len = entry.get('l', 0) or 0
                total = s_off + s_len
                if fid not in sid_min_offset or total < sid_min_offset[fid]:
                    sid_min_offset[fid] = total
                if fid not in sid_max_offset or total > sid_max_offset[fid]:
                    sid_max_offset[fid] = total

    # Step 4: read candidate files.
    file_contents: dict[str, str] = {}
    for path in candidates:
        full = artifacts_dir.parent / path
        try:
            file_contents[path] = full.read_text()
        except (OSError, UnicodeDecodeError):
            pass

    # Step 5: greedily assign unknown source IDs.
    # Use the minimum offset range: if the smallest (s+l) for a source ID
    # exceeds the file size, no entry fits. Otherwise, accept if exactly one
    # candidate matches. A file can map to multiple source IDs across
    # different compilation units, so we only exclude the artifact's own file
    # (already known to belong to its own source_id).
    unknown_ids = sorted(used_ids - id_map.keys())
    for uid in unknown_ids:
        min_off = sid_min_offset.get(uid, 0)
        if min_off == 0:
            continue
        fits = [p for p in candidates
                if p in file_contents and min_off <= len(file_contents[p])]
        # Exclude only the artifact's own file; other files can match
        # multiple source IDs from different compilation units.
        available = [p for p in fits if p != own_path]
        if len(available) == 1:
            id_map[uid] = available[0]

    return id_map


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
    filter_ids: set[int] | None = None,
    use_deployed: bool = True,
) -> str:
    """Generate a markdown table from a Foundry artifact and its project.

    Args:
        artifacts_dir: Path to Foundry's ``out/`` directory.
        artifact_data: Parsed artifact JSON.
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
        ['cast', 'disassemble', bc],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        return f"Error: cast disassemble failed:\n{result.stderr}"
    lines = result.stdout.strip().split('\n')

    id_map = build_source_id_map(artifacts_dir, artifact_data)

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

    print(generate_table(args.artifacts_dir, artifact_data, filter_ids, use_deployed))


if __name__ == '__main__':
    main()
