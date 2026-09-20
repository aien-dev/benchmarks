#!/usr/bin/env python3
"""
Validation script for 32K Shared-Prefix Branching Benchmark Manifest.
Verifies schema, token counts, hashes, categories, and physical safetensors files on disk.
"""

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

DEFAULT_MODEL_DIR = "/home/drakestapleton/.cache/huggingface/hub/models--Qwen--Qwen2.5-7B-Instruct/snapshots/a09a35458c702b33eeacc393d103063234e8bc28"

def hash_tokens(token_ids):
    hasher = hashlib.sha256()
    for tid in token_ids:
        hasher.update(tid.to_bytes(4, byteorder="little", signed=False))
    return hasher.hexdigest()

def hash_file_sha256(filepath: Path) -> str:
    hasher = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(64 * 1024 * 1024):
            hasher.update(chunk)
    return hasher.hexdigest()

def main():
    parser = argparse.ArgumentParser(description="Verify 32K Manifest Integrity and Model Weights")
    parser.add_argument("--manifest", type=str, required=True, help="Path to manifest JSON")
    parser.add_argument(
        "--model-dir",
        type=str,
        default=DEFAULT_MODEL_DIR,
        help="Path to directory containing safetensors shards to physically verify",
    )
    parser.add_argument(
        "--skip-weight-hashes",
        action="store_true",
        help="Skip physical on-disk file hashing of multi-gigabyte safetensors shards",
    )
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
        print(f"Error: Prefix length mismatch: {len(prefix_ids)} vs {data['prefix_token_count']}", file=sys.stderr)
        sys.exit(1)

    if len(prefix_ids) != 32768:
        print(f"Error: Prefix token count is {len(prefix_ids)}, expected 32768", file=sys.stderr)
        sys.exit(1)

    computed_prefix_hash = hash_tokens(prefix_ids)
    if computed_prefix_hash != data["prefix_token_hash"]:
        print(f"Error: Prefix hash mismatch: {computed_prefix_hash} vs {data['prefix_token_hash']}", file=sys.stderr)
        sys.exit(1)

    # 3. Branch validation
    branches = data["branches"]
    if len(branches) != data["branch_count"]:
        print(f"Error: Branch count mismatch: {len(branches)} vs {data['branch_count']}", file=sys.stderr)
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

    # 4. Weight hashes validation (Physical on-disk verification)
    model_dir = Path(args.model_dir)
    weight_hashes = data["weight_hashes"]
    if len(weight_hashes) != 4:
        print(f"Error: Expected 4 weight shard hashes in manifest, found {len(weight_hashes)}", file=sys.stderr)
        sys.exit(1)

    verified_shards = 0
    if not args.skip_weight_hashes:
        if not model_dir.exists():
            print(f"Error: Model directory {model_dir} does not exist for physical shard verification.", file=sys.stderr)
            sys.exit(1)
        print(f"Physically verifying on-disk safetensors hashes in {model_dir}...")
        for shard_name, expected_hash in weight_hashes.items():
            shard_path = model_dir / shard_name
            if not shard_path.exists():
                print(f"Error: Missing physical shard file: {shard_path}", file=sys.stderr)
                sys.exit(1)
            actual_hash = hash_file_sha256(shard_path)
            if actual_hash != expected_hash:
                print(f"Error: Shard {shard_name} hash mismatch: {actual_hash} vs {expected_hash}", file=sys.stderr)
                sys.exit(1)
            print(f"  ✓ {shard_name}: {actual_hash[:16]}... matched")
            verified_shards += 1
        if verified_shards != len(weight_hashes):
            print(f"Error: Expected {len(weight_hashes)} verified shards, got {verified_shards}", file=sys.stderr)
            sys.exit(1)

    print(f"✓ Manifest integrity verified: {manifest_path}")
    print(f"  Model: {data['model_id']} (rev {data['model_revision'][:8]})")
    print(f"  Prefix tokens: {data['prefix_token_count']} (SHA-256: {data['prefix_token_hash'][:16]}...)")
    print(f"  Branches: {data['branch_count']} across {len(categories)} categories: {sorted(categories)}")
    if verified_shards > 0:
        print(f"  Physical safetensors shards verified: {verified_shards}/{len(weight_hashes)} on disk")
    else:
        print(f"  Weight shard manifest entries: {len(weight_hashes)} declared")

if __name__ == "__main__":
    main()
