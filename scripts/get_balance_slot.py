#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["pycryptodomex"]
# ///
"""Find the storage slot responsible for an ERC20 token balance.

Based on the slot20 algorithm by @kendricktan:
https://github.com/kendricktan/slot20
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.request
from typing import Any

from Cryptodome.Hash import keccak

# -- ERC20 function selectors -------------------------------------------------

BALANCE_OF_SELECTOR = "0x70a08231"
DECIMALS_SELECTOR = "0x313ce567"
SYMBOL_SELECTOR = "0x95d89b41"

# Maximum JSON-RPC calls per batch (avoid overwhelming the node).
BATCH_CHUNK_SIZE = 200


# -- helpers ------------------------------------------------------------------

def _pad32(hex_str: str) -> bytes:
    """Left-pad a hex string (with or without 0x prefix) to 32 bytes."""
    raw = hex_str[2:] if hex_str.startswith("0x") else hex_str
    return bytes.fromhex(raw.rjust(64, "0"))


def _keccak256(data: bytes) -> bytes:
    k = keccak.new(digest_bits=256)
    k.update(data)
    return k.digest()


def _abi_encode_address(addr: str) -> bytes:
    """Encode an Ethereum address as an ABI uint256 (32 bytes, left-zero-padded)."""
    return _pad32(addr)


def _storage_key_solidity(holder: str, slot: int) -> str:
    """Compute the storage key for a Solidity-style mapping(key, slot)."""
    key = _pad32(holder)
    slot_bytes = slot.to_bytes(32, "big")
    return "0x" + _keccak256(key + slot_bytes).hex()


def _storage_key_vyper(holder: str, slot: int) -> str:
    """Compute the storage key for a Vyper-style mapping(slot, key)."""
    slot_bytes = slot.to_bytes(32, "big")
    key = _pad32(holder)
    return "0x" + _keccak256(slot_bytes + key).hex()


# -- JSON-RPC -----------------------------------------------------------------

def _rpc_request(url: str, payload: Any) -> Any:
    """Send a single or batch JSON-RPC request and return the decoded body."""
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "slot20/0.1 (Python urllib)",
        },
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def _rpc_single(url: str, method: str, params: list[Any]) -> Any:
    """Make a single JSON-RPC call and return the result."""
    body = _rpc_request(url, {
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
        "id": 1,
    })
    if "error" in body:
        raise RuntimeError(f"RPC error: {body['error']}")
    return body["result"]


def _rpc_batch(url: str, calls: list[tuple[str, list[Any]]]) -> list[Any]:
    """Make a batch of JSON-RPC calls, returning results in order."""
    payload = [
        {"jsonrpc": "2.0", "method": m, "params": p, "id": i}
        for i, (m, p) in enumerate(calls)
    ]
    results: list[Any] = _rpc_request(url, payload)
    # Reorder by id to preserve input order (responses may arrive out of order).
    results.sort(key=lambda r: r["id"])
    return [r["result"] for r in results]


# -- chain helpers ------------------------------------------------------------

def _eth_call(url: str, to: str, data: str) -> str:
    """Execute an eth_call and return the hex result."""
    return _rpc_single(url, "eth_call", [{"to": to, "data": data}, "latest"])


def _eth_call_batch(url: str, calls: list[tuple[str, str]]) -> list[str]:
    """Batch multiple eth_call requests."""
    rpc_calls = [
        ("eth_call", [{"to": to, "data": data}, "latest"])
        for to, data in calls
    ]
    return _rpc_batch(url, rpc_calls)


def _eth_get_storage_at_batch(url: str, token: str, slot_keys: list[str]) -> list[str]:
    """Batch eth_getStorageAt requests, chunking to respect BATCH_CHUNK_SIZE."""
    all_results: list[str] = []
    for i in range(0, len(slot_keys), BATCH_CHUNK_SIZE):
        chunk = slot_keys[i : i + BATCH_CHUNK_SIZE]
        calls = [("eth_getStorageAt", [token, sk, "latest"]) for sk in chunk]
        all_results.extend(_rpc_batch(url, calls))
    return all_results


# -- main logic ---------------------------------------------------------------

def find_slot(
    token: str,
    holder: str,
    rpc_url: str,
    max_slot: int,
    verbose: bool,
) -> int:
    """Return the storage slot number, or -1 if not found."""

    # --- Phase 1: fetch token metadata + holder balance ----------------------
    to = token.lower()
    holder_enc = "0x" + _abi_encode_address(holder).hex()

    # balanceOf, decimals, symbol in one batch
    batch_calls: list[tuple[str, str]] = [
        (to, BALANCE_OF_SELECTOR + holder_enc[2:]),
        (to, DECIMALS_SELECTOR),
        (to, SYMBOL_SELECTOR),
    ]
    results = _eth_call_batch(rpc_url, batch_calls)
    bal_hex, decimals_hex, symbol_hex = results

    holder_bal = int(bal_hex, 16) if bal_hex != "0x" else 0

    if holder_bal == 0:
        if verbose:
            print(f"Token holder {holder} does not hold any tokens", file=sys.stderr)
        return -1

    # Decode decimals (uint8) and symbol (string) for verbose output.
    token_decimals = 18
    token_symbol = token
    try:
        token_decimals = int(decimals_hex, 16)
    except (ValueError, TypeError):
        pass
    try:
        # symbol is a dynamic ABI string: offset (32B) + length (32B) + data (padded to 32B)
        raw = bytes.fromhex(symbol_hex[2:])
        if len(raw) >= 64:
            str_len = int(raw[32:64].hex(), 16)
            token_symbol = bytes(raw[64 : 64 + str_len]).decode("utf-8")
    except (ValueError, TypeError, UnicodeDecodeError):
        pass

    if verbose:
        if token_decimals != 18:
            display = f"{holder_bal / (10 ** token_decimals):.{token_decimals}f}"
        else:
            display = f"{holder_bal / 1e18:.18f}".rstrip("0").rstrip(".")
        print(
            f"Token holder {holder} holds {display} {token_symbol} tokens",
            file=sys.stderr,
        )

    # --- Phase 2: check Solidity mapping format (key, slot) ------------------
    if verbose:
        print(
            f"Checking {token_symbol} slots with Solidity mapping format (key, slot)...",
            file=sys.stderr,
        )

    sol_slot_keys = [_storage_key_solidity(holder, i) for i in range(max_slot + 1)]
    sol_storage = _eth_get_storage_at_batch(rpc_url, to, sol_slot_keys)

    for i, val_hex in enumerate(sol_storage):
        try:
            val = int(val_hex, 16) if val_hex != "0x" else 0
        except (ValueError, TypeError):
            continue
        if val == holder_bal:
            if verbose:
                print(
                    f"Slot number {i} matches balanceOf for {token_symbol} "
                    f"with Solidity mapping format (key, slot)",
                    file=sys.stderr,
                )
            return i
        if verbose:
            # Progress dots every 50 slots (or use a spinner-like indicator).
            if i % 50 == 0 and i > 0:
                print(f"  ... checked {i} slots", file=sys.stderr)

    if verbose:
        print(
            f"No slot found with Solidity mapping format (key, slot)",
            file=sys.stderr,
        )

    # --- Phase 3: check Vyper mapping format (slot, key) ---------------------
    if verbose:
        print(
            f"Checking {token_symbol} slots with Vyper mapping format (slot, key)...",
            file=sys.stderr,
        )

    vyper_slot_keys = [_storage_key_vyper(holder, i) for i in range(max_slot + 1)]
    vyper_storage = _eth_get_storage_at_batch(rpc_url, to, vyper_slot_keys)

    for i, val_hex in enumerate(vyper_storage):
        try:
            val = int(val_hex, 16) if val_hex != "0x" else 0
        except (ValueError, TypeError):
            continue
        if val == holder_bal:
            if verbose:
                print(
                    f"Slot number {i} matches balanceOf for {token_symbol} "
                    f"with Vyper mapping format (slot, key)",
                    file=sys.stderr,
                )
            return i
        if verbose:
            if i % 50 == 0 and i > 0:
                print(f"  ... checked {i} slots", file=sys.stderr)

    if verbose:
        print(
            "No slot found. Try increasing the limit with --limit",
            file=sys.stderr,
        )
    return -1


# -- CLI ----------------------------------------------------------------------

def _validate_address(addr: str) -> str:
    """Basic Ethereum address validation."""
    addr = addr.strip()
    if addr.startswith("0x"):
        addr = addr[2:]
    if len(addr) != 40:
        raise argparse.ArgumentTypeError(
            f"Invalid address: expected 40 hex chars, got {len(addr)}"
        )
    try:
        int(addr, 16)
    except ValueError:
        raise argparse.ArgumentTypeError(f"Invalid address: not hex")
    return "0x" + addr.lower()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Find the slot responsible for the balance of an ERC20 token",
    )
    parser.add_argument(
        "token",
        metavar="token address",
        type=_validate_address,
        help="Address of the ERC20 token",
    )
    parser.add_argument(
        "holder",
        metavar="token holder",
        type=_validate_address,
        help="An address which holds a non-zero amount of the ERC20 token",
    )
    parser.add_argument(
        "--rpc",
        default="https://ethereum-rpc.publicnode.com",
        help="Node RPC URL (default: https://ethereum-rpc.publicnode.com)",
    )
    parser.add_argument(
        "-l", "--limit",
        type=int,
        default=100,
        help="Checks until slot number (default: 100)",
    )
    parser.add_argument(
        "-v", "--verbose",
        action="store_true",
        help="Verbose output",
    )
    args = parser.parse_args()

    try:
        slot = find_slot(
            token=args.token,
            holder=args.holder,
            rpc_url=args.rpc,
            max_slot=args.limit,
            verbose=args.verbose,
        )
    except Exception as e:
        print(f"Error: {e}", file=sys.stderr)
        sys.exit(1)

    # Print the slot number (or -1) to stdout — this is the machine-readable
    # output that the slot20 contract expects.
    print(slot)


if __name__ == "__main__":
    main()
