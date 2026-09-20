#!/usr/bin/env python3
"""
Immutable Manifest Generator for 32K Shared-Prefix Real-Model Branching Benchmark.
Canonical Target: Qwen/Qwen2.5-7B-Instruct (BF16, pinned revision a09a35458c702b33eeacc393d103063234e8bc28).
Hardware Target: NVIDIA DGX Spark (GB10 Grace Blackwell, 128 GB Unified Memory).
"""

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path
from typing import Dict, List, Any

try:
    import torch
    from transformers import AutoTokenizer, AutoModelForCausalLM
except ImportError:
    print("Error: PyTorch and Transformers must be installed in active environment.")
    sys.exit(1)

MODEL_ID = "Qwen/Qwen2.5-7B-Instruct"
PINNED_REVISION = "a09a35458c702b33eeacc393d103063234e8bc28"
PREFIX_TOKEN_COUNT = 32768
BRANCH_COUNT = 500
OUTPUT_TOKEN_LIMIT = 256

WEIGHT_HASHES = {
    "model-00001-of-00004.safetensors": "a1333e6293854747c481288ea83b348226af178dd565c49b6f9495ba1966aba7",
    "model-00002-of-00004.safetensors": "f5d25a2772cb825164a2a2c0fb6d51a87e282abf21e4dd75bc5cfb3cd0ea6185",
    "model-00003-of-00004.safetensors": "8efdec4c1bc12317ae1a38dc42b595ce777738a64deea3fcb8a0a91381bcdfd5",
    "model-00004-of-00004.safetensors": "1a72d403cdf0c1ec3cb7f289f17b394a01e64394c2e9b3c0f94dbce3faf879bd",
}

CATEGORIES = [
    "factual_extraction",
    "summarization",
    "code_transformation",
    "multi_step_continuation",
    "structured_json",
]

def hash_tokens(token_ids: List[int]) -> str:
    hasher = hashlib.sha256()
    for tid in token_ids:
        hasher.update(tid.to_bytes(4, byteorder="little", signed=False))
    return hasher.hexdigest()

def build_long_prefix_text() -> str:
    base_text = """
### Sovereign Agent System Architecture & Autonomous Protocol Specification
The AIEN sovereign runtime executes high-concurrency multi-agent trees over coherent unified memory on NVIDIA Grace Blackwell GB10.
Every agent branch maintains an immutable lineage reference to the parent context. When a subagent forks to investigate a divergent hypothesis,
its key-value state is managed via Copy-on-Write (COW) memory pages. Physical allocation is deferred until a write fault occurs on the branch suffix.

Key System Invariants:
1. Unified Memory Coherence: GB10 provides 128 GB coherent unified LPDDR5x memory shared between the 72-core Neoverse-V2 CPU and Blackwell GPU.
2. Zero Page Cache Leakage: Linux page cache pressure must be cleared before allocating device-resident memory blocks.
3. Cryptographic Verification: Every execution receipt is signed with the hardware TPM 2.0 key bound to /dev/tpmrm0.
4. Non-Interference Isolation: Subprocess execution occurs within dual-jail namespace containment to guarantee deterministic benchmark isolation.

Module 01: Paged Memory Allocator & Virtual Block Table
The paged KV cache allocator partitions memory into fixed-size 16-token and 32-token blocks.
Each sequence maintains a block table mapping virtual sequence indices to physical block addresses.
Atomic reference counting governs block lifecycle:
- Prefill Block Allocation: A sequence of N tokens allocates ceil(N / BlockSize) contiguous physical blocks with refcount = 1.
- Branch Fork: When sequence S spawns child sequences C_1 .. C_k, the child block tables receive copies of the parent block descriptors,
  and each shared block increments its atomic reference count by k.
- Autoregressive Divergence: When child C_i appends a generated token, if the active tail block has refcount > 1,
  a new physical block is allocated from the free list, populated with the tail tokens, and swapped into C_i block table.

Module 02: Hardware Tensor Core Execution & NVFP4 Quantization
Blackwell Tensor Cores support native FP4 (E2M1) with per-block micro-scaling (block size 16 elements).
Weights are stored in packed 4-bit representations alongside FP8 or BF16 scaling vectors.
Activations are dynamically quantized per 16-element block on ingress.
Paged attention kernels directly read packed FP4 key and value pages from unified memory, dequantizing to registers on-the-fly.

Module 03: Telemetry Collection & Sampling Harness
High-frequency sampling monitors system-wide MemAvailable from /proc/meminfo and per-process Proportional Set Size (Pss)
and Resident Set Size (Rss) from /proc/<pid>/smaps_rollup at 50ms intervals.
NVML telemetry captures GPU core temperature, power consumption (Watts), clock frequencies, and active compute utilization.
Every trial yields an immutable receipt containing token timestamps, inter-token latencies, time-to-first-token (TTFT),
and energy efficiency measured in Joules per generated token.
"""
    return base_text

def generate_branch_suffixes(tokenizer: Any, count: int) -> List[Dict[str, Any]]:
    suffixes = []
    for i in range(count - 100):
        cat = CATEGORIES[i % len(CATEGORIES)]
        idx = i // len(CATEGORIES)
        if cat == "factual_extraction":
            text = f"Branch {i:04d}: Extract the precise hardware memory bandwidth and invariant rules from Section {idx:02d}."
        elif cat == "summarization":
            text = f"Branch {i:04d}: Summarize the block allocation lifecycle and atomic reference counting for module {idx:02d}."
        elif cat == "code_transformation":
            text = f"Branch {i:04d}: Refactor the paged attention memory access loop in kernel {idx:02d} to minimize cache faults."
        elif cat == "multi_step_continuation":
            text = f"Branch {i:04d}: Trace the hypothesis that lock contention at branch step {idx:02d} causes p99 latency spikes."
        else:
            text = f"Branch {i:04d}: Emit a valid JSON schema specifying telemetry fields for hardware monitor {idx:02d}."
        
        tids = tokenizer.encode(text, add_special_tokens=False)
        suffixes.append({
            "branch_id": i,
            "category": cat,
            "text": text,
            "token_ids": tids,
            "token_count": len(tids),
        })
        
    vocab_size = tokenizer.vocab_size
    step = vocab_size // 120
    for j in range(100):
        branch_idx = (count - 100) + j
        synthetic_tids = [(1000 + (j * 73) + (k * step)) % vocab_size for k in range(20)]
        text = tokenizer.decode(synthetic_tids)
        suffixes.append({
            "branch_id": branch_idx,
            "category": "synthetic_divergence",
            "text": text,
            "token_ids": synthetic_tids,
            "token_count": len(synthetic_tids),
        })
        
    return suffixes

def main():
    parser = argparse.ArgumentParser(description="Generate 32K Shared-Prefix Branching Benchmark Manifest")
    parser.add_argument("--output", type=str, default="/home/drakestapleton/workspace/benchmarks/manifests/qwen25_7b_branching_32k.json")
    parser.add_argument("--oracle-branches", type=int, default=0, help="Number of branches to run reference oracle for (0 to skip)")
    args = parser.parse_args()

    print(f"[Manifest] Initializing tokenizer for {MODEL_ID} revision {PINNED_REVISION}...")
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=PINNED_REVISION)

    print("[Manifest] Building 32K shared prefix...")
    base_text = build_long_prefix_text()
    full_text = base_text
    while len(tokenizer.encode(full_text, add_special_tokens=False)) < PREFIX_TOKEN_COUNT:
        full_text += "\n" + base_text
    
    encoded_prefix = tokenizer.encode(full_text, add_special_tokens=False)
    prefix_tids = encoded_prefix[:PREFIX_TOKEN_COUNT]
    prefix_hash = hash_tokens(prefix_tids)
    print(f"[Manifest] 32K Prefix token count: {len(prefix_tids)}, SHA-256: {prefix_hash}")

    print(f"[Manifest] Generating {BRANCH_COUNT} deterministic branch suffixes...")
    branch_specs = generate_branch_suffixes(tokenizer, BRANCH_COUNT)

    for b in branch_specs:
        input_tids = prefix_tids + b["token_ids"]
        b["input_token_count"] = len(input_tids)
        b["input_token_hash"] = hash_tokens(input_tids)
        b["expected_first_token"] = None
        b["expected_first_16_tokens"] = []
        b["expected_full_tokens"] = []
        b["expected_output_hash"] = None
        b["expected_output_text"] = ""

    if args.oracle_branches > 0:
        n_oracle = min(args.oracle_branches, BRANCH_COUNT)
        print(f"[Manifest] Loading {MODEL_ID} in BF16 onto CUDA for {n_oracle} oracle branches...")
        model = AutoModelForCausalLM.from_pretrained(
            MODEL_ID,
            revision=PINNED_REVISION,
            dtype=torch.bfloat16
        ).to("cuda")
        model.eval()

        print(f"[Manifest] Computing oracle completions (max_new_tokens={OUTPUT_TOKEN_LIMIT})...")
        with torch.no_grad():
            for idx in range(n_oracle):
                b = branch_specs[idx]
                input_tids = prefix_tids + b["token_ids"]
                input_tensor = torch.tensor([input_tids], device="cuda")
                t0 = time.time()
                outputs = model.generate(
                    input_tensor,
                    max_new_tokens=OUTPUT_TOKEN_LIMIT,
                    do_sample=False,
                    pad_token_id=tokenizer.eos_token_id,
                )
                dt = time.time() - t0
                generated_ids = outputs[0][len(input_tids):].tolist()
                b["expected_first_token"] = generated_ids[0] if generated_ids else None
                b["expected_first_16_tokens"] = generated_ids[:16]
                b["expected_full_tokens"] = generated_ids
                b["expected_output_hash"] = hash_tokens(generated_ids)
                b["expected_output_text"] = tokenizer.decode(generated_ids, skip_special_tokens=True)
                print(f"  Oracle Branch {b["branch_id"]:04d} ({b["category"]}): {len(generated_ids)} tokens in {dt:.2f}s")

    manifest = {
        "manifest_version": "1.0.0",
        "benchmark_suite": "aien-branching-32k",
        "created_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "model_id": MODEL_ID,
        "model_revision": PINNED_REVISION,
        "weight_hashes": WEIGHT_HASHES,
        "prefix_token_count": len(prefix_tids),
        "prefix_token_hash": prefix_hash,
        "prefix_token_ids": prefix_tids,
        "branch_count": len(branch_specs),
        "generation_config": {
            "temperature": 0.0,
            "do_sample": False,
            "max_new_tokens": OUTPUT_TOKEN_LIMIT,
        },
        "branches": branch_specs,
    }

    out_path = Path(args.output)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w") as f:
        json.dump(manifest, f, indent=2)

    print(f"[Manifest] Successfully generated manifest: {out_path} ({os.path.getsize(out_path) / (1024*1024):.2f} MB)")

if __name__ == "__main__":
    main()
