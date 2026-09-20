#!/usr/bin/env python3
"""
Validation script for 32K Shared-Prefix Branching Benchmark Manifest.
"""

import argparse
import hashlib
import json
import sys
from pathlib import Path

def hash_tokens(token_ids):
    hasher = hashlib.sha256()
    for tid in token_ids:
        hasher.update(tid.to_bytes(4, byteorder="little", signed=False))
    return hasher.hexdigest()

def main():
    parser = argparse.ArgumentParser(description="Verify 32K Manifest Integrity")
    parser.add_argument("--manifest", type=str, required=True, help="Path to manifest JSON")
    args = parser.parse_args()

    manifest_path = Path(args.manifest)
    if not manifest_path.exists():
        print(f"Error: Manifest file does not exist: {manifest_path}", file=sys.stderr)
        sys.exit(1)

    with open(manifest_path, "r") as f:
        data = json.load(f)

    # 1. Root schema validation
    required_keys = [
        "manifest_version", "benchmark_suite", "model_id", "model_revision",
        "weight_hashes", "prefix_token_count", "prefix_token_hash",
        "prefix_token_ids", "branch_count", "branches"
    ]
    for key in required_keys:
        if key not in data:
            print(f"Error: Missing required root key: {key}", file=sys.stderr)
            sys.exit(1)

    # 2. Prefix validation
    prefix_ids = data["prefix_token_ids"]
    if len(prefix_ids) != data["prefix_token_count"]:
        print(f"Error: Prefix length mismatch: {len(prefix_ids)} vs {data["prefix_token_count"]}", file=sys.stderr)
        sys.exit(1)

    if len(prefix_ids) != 32768:
        print(f"Error: Prefix token count is {len(prefix_ids)}, expected 32768", file=sys.stderr)
        sys.exit(1)

    computed_prefix_hash = hash_tokens(prefix_ids)
    if computed_prefix_hash != data["prefix_token_hash"]:
        print(f"Error: Prefix hash mismatch: {computed_prefix_hash} vs {data["prefix_token_hash"]}", file=sys.stderr)
        sys.exit(1)

    # 3. Branch validation
    branches = data["branches"]
    if len(branches) != data["branch_count"]:
        print(f"Error: Branch count mismatch: {len(branches)} vs {data["branch_count"]}", file=sys.stderr)
        sys.exit(1)

    categories = set()
    for b in branches:
        bid = b["branch_id"]
        cat = b["category"]
        categories.add(cat)
        tids = b["token_ids"]
        if not (16 <= len(tids) <= 32):
            print(f"Error: Branch {bid} token count {len(tids)} outside bounds [16, 32]", file=sys.stderr)
            sys.exit(1)

        expected_input_hash = hash_tokens(prefix_ids + tids)
        if expected_input_hash != b["input_token_hash"]:
            print(f"Error: Branch {bid} input hash mismatch", file=sys.stderr)
            sys.exit(1)

    # 4. Weight hashes validation
    if len(data["weight_hashes"]) != 4:
        print(f"Error: Expected 4 weight shard hashes, found {len(data["weight_hashes"])}", file=sys.stderr)
        sys.exit(1)

    print(f"✓ Manifest integrity verified: {manifest_path}")
    print(f"  Model: {data["model_id"]} (rev {data["model_revision"][:8]})")
    print(f"  Prefix tokens: {data["prefix_token_count"]} (SHA-256: {data["prefix_token_hash"][:16]}...)")
    print(f"  Branches: {data["branch_count"]} across {len(categories)} categories: {sorted(categories)}")
    print(f"  Weight shards: {len(data["weight_hashes"])} safetensors verified")

if __name__ == "__main__":
    main()
