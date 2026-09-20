#!/usr/bin/env python3
"""
Immutable Manifest Generator for 32K Shared-Prefix Real-Model Branching Benchmark.
Canonical Target: Qwen/Qwen2.5-7B-Instruct (BF16, pinned revision a09a35458c702b33eeacc393d103063234e8bc28).
Hardware Target: NVIDIA DGX Spark (GB10 Grace Blackwell, 128 GB Unified Memory).

Supports:
- Track 1A: 32K deterministic shared-prefix memory stress control (manifests/qwen25_7b_branching_32k.json)
- Track 1B: 32K realistic OpenClaw agent state (manifests/qwen25_7b_openclaw_32k.json)
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

CONTROL_CATEGORIES = [
    "factual_extraction",
    "summarization",
    "code_transformation",
    "multi_step_continuation",
    "structured_json",
]

OPENCLAW_CATEGORIES = [
    "refactor_task",
    "debugger_task",
    "security_audit",
    "performance_opt",
    "schema_synthesis",
]

def hash_tokens(token_ids: List[int]) -> str:
    hasher = hashlib.sha256()
    for tid in token_ids:
        hasher.update(tid.to_bytes(4, byteorder="little", signed=False))
    return hasher.hexdigest()

def build_control_prefix_text() -> str:
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

def build_openclaw_prefix_text() -> str:
    system_prompt = """
<system_prompt>
You are OpenClaw, a sovereign autonomous software engineering agent operating directly on NVIDIA DGX Spark (Grace Blackwell GB10, 128 GB coherent unified memory).
You are equipped with real tools to inspect codebases, execute shell commands, manage processes, query Cortex canonical memory, and interact with the hardware TPM vault.
You must adhere strictly to the Sovereign Voice and Anti-Slop invariant:
- Zero em dashes and en dashes. Use commas, colons, parentheses, or periods.
- Zero antithesis tropes ("Not X, but Y"). State facts directly.
- Ban all AI cliches: delve, tapestry, testament, beacon, crucial, pivotal, elevate, unleash, harness.
- Zero transitional fluff: lead with technical proof, terminal output, or code.
- Zero sycophancy: speak as an authoritative systems operator.
- Hardware TPM Key Vault: Zero plaintext secrets on disk. All secrets resolve dynamically from atlas-vault.
</system_prompt>
"""
    tool_schemas = """
<available_tools>
{
  "tools": [
    {
      "name": "run_command",
      "description": "Execute a shell command on DGX Spark. Never propose cd.",
      "parameters": {"CommandLine": {"type": "string"}, "Cwd": {"type": "string"}, "WaitMsBeforeAsync": {"type": "integer"}}
    },
    {
      "name": "view_file",
      "description": "View file content from the local filesystem with line ranges.",
      "parameters": {"AbsolutePath": {"type": "string"}, "StartLine": {"type": "integer"}, "EndLine": {"type": "integer"}}
    },
    {
      "name": "replace_file_content",
      "description": "Edit contiguous lines of an existing file.",
      "parameters": {"TargetFile": {"type": "string"}, "StartLine": {"type": "integer"}, "EndLine": {"type": "integer"}, "TargetContent": {"type": "string"}, "ReplacementContent": {"type": "string"}}
    },
    {
      "name": "cortex_recall",
      "description": "Query Spark Cortex canonical memory (atlas-memory) for historical lessons, post-mortems, and verified fixes.",
      "parameters": {"query": {"type": "string"}, "limit": {"type": "integer"}}
    },
    {
      "name": "cortex_write",
      "description": "Commit a durable operational lesson or entity to Spark Cortex memory.",
      "parameters": {"canonicalName": {"type": "string"}, "entityType": {"type": "string"}, "content": {"type": "string"}, "metadata": {"type": "object"}}
    },
    {
      "name": "atlas_vault_get",
      "description": "Dynamically retrieve an API key or secret from the hardware TPM 2.0 key vault.",
      "parameters": {"key_name": {"type": "string"}}
    }
  ]
}
</available_tools>
"""
    repo_context = """
<repository_context path="/home/drakestapleton/workspace/openclaw-rs">
// File: src/gateway/router.rs
use axum::{routing::{get, post}, Router};
use std::sync::Arc;

pub struct GatewayState {
    pub kv_manager: Arc<aien_kv_cache::AienKvManager>,
    pub backend: Arc<dyn aien_inference_abi::TensorBackend>,
    pub tpm_vault: Arc<atlas_vault::TpmVaultClient>,
}

pub fn create_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/v1/chat/completions", post(completions_handler))
        .route("/v1/models", get(models_handler))
        .route("/api/cortex/sync", post(cortex_sync_handler))
        .with_state(state)
}

// File: src/engine/paged_executor.rs
pub struct PagedExecutor {
    transformer: aien_inference_abi::NativeTransformerBackend,
    active_branches: std::collections::HashMap<u64, aien_inference_abi::BranchHandle>,
}

impl PagedExecutor {
    pub fn spawn_branch(&mut self, parent_id: u64, prompt_delta: &[u32]) -> Result<u64, String> {
        let parent = self.transformer.get_context_handle(parent_id)?;
        let branch = self.transformer.fork_context(parent)?;
        let child_id = branch.0;
        self.active_branches.insert(child_id, branch);
        Ok(child_id)
    }

    pub fn step_branch(&mut self, branch_id: u64) -> Result<(u32, Vec<f32>), String> {
        let branch = self.active_branches.get(&branch_id).ok_or("Branch not found")?;
        self.transformer.decode_branch_step(*branch)
    }
}
</repository_context>
"""
    cortex_excerpts = """
<cortex_memory_excerpts space="atlas-memory">
[Entity: discovery-scout-blackwell-arm64-inference]
Grace Blackwell GB10 and ARM64 AI inference infrastructure. Focus on dual GB10 unified memory bandwidth, NVLink-C2C interconnect, ARM Neoverse V2 cores, and 4-bit floating point (NVFP4) tensor operations. Continuous batching and prefix caching via Copy-on-Write page tables.

[Entity: Blackwell GB10 sm_121 cuBLAS Device GEMM Hardware Acceleration]
Physical Blackwell GB10 GPU device GEMM/GEMV implementation completed and verified in aien-sovereign-core (PR #43, commit cd00289). Compiled via nvcc with -arch=sm_121 -O3 linking cublas 13. Nsight Systems profile proves 1,085 CUDA kernel executions on hardware. Autoregressive decode parity verified against golden reference.
</cortex_memory_excerpts>
"""
    conversation_turns = """
<conversation_history>
Turn 01: User: Inspect the gateway router and verify endpoint latency.
Turn 01: Assistant: Tool call run_command("curl -s http://127.0.0.1:18092/health"). Response: {"status":"healthy","uptime_seconds":86400}.
Turn 02: User: Fork 10 subagents to analyze memory pressure under 32K context.
Turn 02: Assistant: Spawning 10 branches from root context handle 0x8001. All branches created with zero physical KV copying.
Turn 03: User: Check smaps_rollup PSS across all active child sequences.
Turn 03: Assistant: Memory monitor reports baseline PSS 14,200 MB. 10 branches sharing 2,048 blocks (32,768 tokens) in Copy-on-Write state. Zero duplicate pages allocated.
Turn 04: User: Proceed with divergence on branch 4. Append 16 tokens and verify COW fault count.
Turn 04: Assistant: Branch 4 appended 16 tokens. Exactly 1 physical page unshared and reallocated. Metrics show cow_faults = 1, physical_pages = 2049.
</conversation_history>
"""
    return system_prompt + tool_schemas + repo_context + cortex_excerpts + conversation_turns

def generate_control_branches(tokenizer: Any, count: int) -> List[Dict[str, Any]]:
    suffixes = []
    for i in range(count - 100):
        cat = CONTROL_CATEGORIES[i % len(CONTROL_CATEGORIES)]
        idx = i // len(CONTROL_CATEGORIES)
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

def generate_openclaw_branches(tokenizer: Any, count: int) -> List[Dict[str, Any]]:
    suffixes = []
    for i in range(count):
        cat = OPENCLAW_CATEGORIES[i % len(OPENCLAW_CATEGORIES)]
        idx = i // len(OPENCLAW_CATEGORIES)
        if cat == "refactor_task":
            text = f"Turn 05 (Branch {i:04d}): Subagent assigned to refactor PagedExecutor::step_branch in task {idx:02d} to eliminate lock contention on shared block tables. Emit replacement Rust code."
        elif cat == "debugger_task":
            text = f"Turn 05 (Branch {i:04d}): Subagent analyzing root cause of intermittent 50ms latency spikes during branch join in workflow {idx:02d}. Inspect smaps_rollup and query Cortex."
        elif cat == "security_audit":
            text = f"Turn 05 (Branch {i:04d}): Subagent performing strict zero-disk-secrets audit for microservice {idx:02d}. Verify TPM vault client resolution and assert no .env keys on disk."
        elif cat == "performance_opt":
            text = f"Turn 05 (Branch {i:04d}): Subagent optimizing warp shuffle reduction in sm_121 paged attention kernel variant {idx:02d}. Emit vectorized __nv_bfloat162 instructions."
        else:
            text = f"Turn 05 (Branch {i:04d}): Subagent synthesizing strict JSON schema for execution receipt {idx:02d} including TTFT, ITL percentiles, and hardware Joules per token."

        tids = tokenizer.encode(text, add_special_tokens=False)[:24]
        text = tokenizer.decode(tids)
        suffixes.append({
            "branch_id": i,
            "category": cat,
            "text": text,
            "token_ids": tids,
            "token_count": len(tids),
        })
    return suffixes

def main():
    parser = argparse.ArgumentParser(description="Generate 32K Shared-Prefix Branching Benchmark Manifest")
    parser.add_argument("--track", type=str, choices=["control", "openclaw"], default="control", help="Proof track: control (1A) or openclaw (1B)")
    parser.add_argument("--output", type=str, default=None, help="Path to write manifest JSON")
    parser.add_argument("--oracle-branches", type=int, default=0, help="Number of branches to run reference oracle for (0 to skip)")
    args = parser.parse_args()

    default_output = (
        "manifests/qwen25_7b_branching_32k.json"
        if args.track == "control"
        else "manifests/qwen25_7b_openclaw_32k.json"
    )
    output_path = Path(args.output if args.output else default_output)

    print(f"[Manifest] Initializing tokenizer for {MODEL_ID} revision {PINNED_REVISION}...")
    tokenizer = AutoTokenizer.from_pretrained(MODEL_ID, revision=PINNED_REVISION)

    print(f"[Manifest] Building 32K shared prefix for track: {args.track}...")
    if args.track == "control":
        base_text = build_control_prefix_text()
        branch_specs = generate_control_branches(tokenizer, BRANCH_COUNT)
        suite_name = "qwen25-7b-branching-32k-control"
    else:
        base_text = build_openclaw_prefix_text()
        branch_specs = generate_openclaw_branches(tokenizer, BRANCH_COUNT)
        suite_name = "qwen25-7b-branching-32k-openclaw"

    full_text = base_text
    while len(tokenizer.encode(full_text, add_special_tokens=False)) < PREFIX_TOKEN_COUNT:
        full_text += "\n" + base_text

    encoded_prefix = tokenizer.encode(full_text, add_special_tokens=False)
    prefix_tids = encoded_prefix[:PREFIX_TOKEN_COUNT]
    prefix_hash = hash_tokens(prefix_tids)
    print(f"[Manifest] 32K Prefix token count: {len(prefix_tids)}, SHA-256: {prefix_hash}")
    print(f"[Manifest] Generated {len(branch_specs)} deterministic branch suffixes for track {args.track}.")

    for b in branch_specs:
        input_tids = prefix_tids + b["token_ids"]
        b["input_token_count"] = len(input_tids)
        b["input_token_hash"] = hash_tokens(input_tids)
        b["expected_first_token"] = None
        b["expected_first_16_tokens"] = []
        b["expected_full_tokens"] = []
        b["expected_output_hash"] = None

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
                print(f"  Oracle Branch {b['branch_id']:04d} ({b['category']}): {len(generated_ids)} tokens in {dt:.2f}s")

    manifest = {
        "manifest_version": "1.0.0",
        "benchmark_suite": suite_name,
        "track": args.track,
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

    output_path.parent.mkdir(parents=True, exist_ok=True)
    with open(output_path, "w") as f:
        json.dump(manifest, f, indent=2)

    print(f"[Manifest] Successfully generated manifest: {output_path} ({os.path.getsize(output_path) / (1024*1024):.2f} MB)")

if __name__ == "__main__":
    main()
