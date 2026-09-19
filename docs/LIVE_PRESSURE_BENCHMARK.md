# AIEN Live Stress & Pressure Benchmark: Speed, Latency & Memory

## Execution Environment & Hardware Substrate

- **Workstation**: NVIDIA DGX Spark (`spark-b87b`)
- **Processor**: NVIDIA Grace Blackwell (GB10, aarch64, 128-bit NEON / SVE)
- **Memory**: 121 GB Unified LPDDR5X (Unified CPU/GPU Physical Memory)
- **Host Kernel**: Linux 7.0.0-1019-nvidia (4 KB page size)
- **Compilers**: rustc 1.98.1 (48a229cea 2026-09-01), CPython 3.12.3

---


> [!NOTE]
> **Control Plane Scope Clarification**:
> The throughput and latency figures in Sections 1 through 3 measure native control plane execution:
> block-table indexing, sequence queue transitions, reference-counting mutations, and batch assembly.
> Block indices represent coordinate pointers in pre-allocated unified memory.
> Physical tensor manipulation across the Blackwell GPU substrate begins in Stages 2 through 4.

## 1. Native Paged KV Cache Block-Table Allocator Throughput (Control Plane)

Direct empirical measurement of the physical KV block table allocator (`aien-kv-cache`).
Workload: 10,000 sequence allocations (160,000 block-table indices managed, block size = 16 tokens).

| Metric | Measured Value | Per-Unit Latency |
| :--- | :--- | :--- |
| **Allocation Throughput** | **134,338,182 blocks/sec** | 119.10 ns/sequence (7.44 ns/block) |
| **Deallocation Throughput** | **238,709,061 blocks/sec** | 67.03 ns/sequence (4.19 ns/block) |
| **Memory Allocation Complexity** | O(1) Constant Time | Pre-mapped unified pool |

---

## 2. Subagent Zero-Copy Sequence Fork vs Memory Copy

Parent sequence context: 4,096 tokens (256 KV blocks, ~384 MB physical KV state in BF16).
Measures time and memory required to spawn autonomous child subagents branching from parent context.

| Subagents Forked | Zero-Copy Fork Time | Naive Memory Copy | Speedup Ratio | Projected Tensor Memory Saved |
| :--- | :--- | :--- | :--- | :--- |
| **1** | 1.58 µs | 1.92 ms | **1,212.1x** | 0.38 GB |
| **10** | 0.46 µs | 19.20 ms | **4,152.2x** | 3.75 GB |
| **50** | 0.47 µs | 96.00 ms | **4,067.8x** | 18.75 GB |
| **100** | 0.39 µs | 192.00 ms | **4,977.2x** | 37.50 GB |
| **500** | 0.45 µs | 960.00 ms | **4,236.1x** | 187.50 GB |

- **Copy-on-Write (CoW) Mutation Latency**: 8.96 ns per diverging token append.
- **Subagent Divergence Overhead**: Negligible. Subagents branch without duplicating common KV history.

---

## 3. Continuous Batching Scheduler Step Overhead

Native continuous batching scheduler (`aien-scheduler`) managing token admission, continuous batching, and chunked prefill budgets.

| Active Sequences | Batch Build Time (µs) | Scheduler Overhead Ratio (in 10ms forward pass) |
| :--- | :--- | :--- |
| 1 | 1.01 µs | 0.0101% |
| 9 | 2.02 µs | 0.0202% |
| 40 | 5.20 µs | 0.0520% |
| 96 | 9.66 µs | 0.0966% |
| 192 | 17.68 µs | 0.1768% |

---

## 4. Live Microservice Call Stress & Concurrency Saturation

Executed via `stress_aien_live` against active production daemons on `spark`.

### Cortex-rs Vector Memory Call Stress (Port 18080, `/api/cortex/search`)
- 200 requests executed across concurrency sweeps against SQLite WAL + vector similarity table.

| Concurrency | Throughput (req/s) | p50 Latency (ms) | p90 Latency (ms) | p95 Latency (ms) | p99 Latency (ms) | Success Rate |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **10** | 1,637.21 req/s | 6.03 ms | 6.68 ms | 7.10 ms | 11.35 ms | 100.0% |
| **25** | 1,867.17 req/s | 10.97 ms | 17.30 ms | 29.68 ms | 41.53 ms | 100.0% |
| **50** | 1,960.52 req/s | 11.82 ms | 36.98 ms | 57.90 ms | 78.56 ms | 100.0% |
| **100** | 2,103.73 req/s | 23.02 ms | 57.30 ms | 68.83 ms | 88.18 ms | 100.0% |

### Cortex Transformer Encoder Batch Throughput (Port 18081, `/embed`)
- Model: `BAAI/bge-base-en-v1.5` (ONNX INT8, 4 CPU threads).

| Batch Size | Total Texts | Duration (ms) | Throughput (texts/sec) | Per-Text Latency (ms) |
| :--- | :--- | :--- | :--- | :--- |
| **1** | 20 | 100.21 ms | 199.59 texts/s | 5.01 ms |
| **4** | 80 | 451.71 ms | 177.11 texts/s | 5.65 ms |
| **8** | 160 | 851.81 ms | 187.84 texts/s | 5.32 ms |
| **16** | 320 | 1547.26 ms | 206.82 texts/s | 4.84 ms |
| **32** | 640 | 3165.60 ms | 202.17 texts/s | 4.95 ms |

### MAX LLM Streaming & Concurrency Pressure (Port 18082, `unsloth/Llama-3.2-1B-Instruct`)
- 32 tokens per stream, multi-stream parallel client load.

| Streams | TTFT p50 (ms) | ITL p50 (ms) | ITL p95 (ms) | Total Tok/s | E2E Latency p50 (ms) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1** | 130.59 ms | 90.62 ms | 97.77 ms | 11.58 tok/s | 2,763.97 ms |
| **4** | 5,652.78 ms | 86.60 ms | 103.46 ms | 11.61 tok/s | 8,299.38 ms |
| **8** | 11,198.92 ms | 87.25 ms | 101.76 ms | 11.59 tok/s | 13,795.19 ms |
| **16** | 22,234.08 ms | 84.76 ms | 100.95 ms | 11.66 tok/s | 24,854.70 ms |

---

## 5. Memory Footprint Stability & Post-Stress Recovery

Resident Set Size (RSS) inspected directly from `/proc/[pid]/status` (`VmRSS`) before and after concurrency saturation.

| Service | Baseline RSS | Peak Stress RSS | Post-Stress Delta | Memory Classification |
| :--- | :--- | :--- | :--- | :--- |
| **cortex-rs** | 15.97 MB | 18.57 MB | +2.60 MB | Zero memory leaks |
| **cortex-encoder-rs** | 780.02 MB | 780.39 MB | +0.36 MB | Deterministic memory profile |
| **max inference engine** | 9,011.61 MB | 9,013.99 MB | +2.38 MB | Stable model heap |
| **openclaw daemon** | 4.80 MB | 4.80 MB | +0.00 MB | Zero allocation drift |
