# AIEN Sovereign Systems Performance Benchmark Suite

Hardware-grounded, reproducible performance measurements of the AIEN Sovereign AI ecosystem executing on dedicated silicon.

![Memory RSS Comparison](assets/memory_comparison.svg)

---

## Hardware Specification

- **Workstation**: NVIDIA DGX Spark (`spark-b87b`)
- **Processor**: NVIDIA Grace Blackwell (GB10, aarch64)
- **Memory**: 128 GB Unified LPDDR5X (Unified CPU/GPU Physical Memory)
- **Host Kernel**: Linux 6.8.0-spark-gb10 (64KB page size)
- **Compiler**: rustc 1.90.0 / LLVM 19, Mojo 24.5 / MAX Engine 24.5

---

## Executive Summary

The AIEN architecture enforces a strict Native Systems Priority: zero Python or Node interpreters across core services, gateways, or background daemons. Core agent services run as pure compiled native Rust and Mojo binaries with in-memory TPM-bound key resolution.

1. **Memory Reduction**: Native Rust daemons reduce resident set size (RSS) by **81% to 99.8%** compared to Python FastAPI, LangChain, or LlamaIndex agent stacks. `openclaw-rs` maintains an autonomous heartbeat loop within **4.78 MB** RSS.
2. **Gateway Latency**: Axum microservices deliver **p50 TTFB of 3.37ms to 4.30ms** at over **2,000 requests/second** under concurrent load (concurrency=10, 500 requests per endpoint).
3. **Local Quantized Vectorization**: `cortex-encoder-rs` executes quantized INT8 bi-encoder embeddings via native ONNX Runtime C-API in **4.09ms** for short queries and **5.78ms** for medium context.
4. **Compiled Mojo SIMD**: Mojo SIMD vector reduction runs in **4.39ms** per invocation including process launch and JSON output.

---

## What These Numbers Mean in Practice

Raw telemetry numbers translate directly into concrete advantages for developers and operators:

### 1. Memory Footprint (4.78 MB vs 3,737 MB)
- **The Problem**: Typical AI agent stacks written in Python (LangChain, AutoGen, CrewAI) consume 3 to 4 gigabytes of host RAM while idling. On a workstation or laptop with 16 GB to 64 GB of memory, this interpreter bloat starves the system.
- **The AIEN Solution**: Core agent daemons run in 4.78 MB to 10.3 MB resident set size. This 99.8% memory reduction leaves virtually all physical host and GPU unified RAM free to load 32B or 70B parameter neural models directly into memory.

### 2. Response Latency (3.56 ms vs 38.4 ms)
- **The Problem**: When an autonomous agent queries memory or dispatches tools, every internal HTTP hop through an interpreted server adds 30 to 50 milliseconds of latency. A chain of ten tool calls introduces half a second of lag before generation begins.
- **The AIEN Solution**: Axum microservices respond in 3.56 milliseconds (p50). To human operators and interacting systems, this latency is imperceptible, enabling instant context retrieval and immediate execution loops.

### 3. Request Throughput (2,056 req/s vs 214 req/s)
- **The Problem**: Python async servers bottleneck under concurrent requests, requiring complex multi-worker clustering or external proxies to sustain basic loads.
- **The AIEN Solution**: A single native Rust process sustains over 2,000 requests per second with flat tail latencies. One machine handles workloads that typically require an entire server rack.

### 4. Local INT8 Vectorization (4.09 ms)
- **The Problem**: External embedding APIs introduce cloud dependencies, recurring subscription costs, and external data exposure.
- **The AIEN Solution**: ONNX Runtime INT8 quantization executes bi-encoder embeddings directly on local silicon in 4.09 milliseconds. Epistemic memory indexing remains fast, private, and offline.

---

## Detailed Telemetry & Comparison Tables

### 1. Memory Resident Set Size (RSS)

Measured on live services via `/proc/[pid]/status` (`VmRSS`) under steady state:

| Service | Architecture | Role | Memory RSS | Footprint Delta vs Python Baseline |
| :--- | :--- | :--- | :--- | :--- |
| **openclaw-rs** | Native Rust (Axum + SQLite) | Autonomous Agent Runtime | **4.78 MB** | **-99.87%** |
| **spark-cockpit-rs** | Native Rust (Axum) | Health & Telemetry Gateway | **8.57 MB** | **-99.77%** |
| **cortex-rs** | Native Rust (Axum + SQLite WAL) | Knowledge Graph Engine | **10.30 MB** | **-99.72%** |
| **cortex-encoder-rs** | Native Rust (ONNX INT8) | Local Vector Embedding | **776.16 MB** | -79.23% (vs PyTorch) |
| *Python Minimal (Uvicorn)* | Python 3.12 / Uvicorn | Single Status Endpoint | 45.29 MB | Baseline (Minimal) |
| *Python Full Agent Stack* | Python 3.12 / FastAPI / PyTorch | Standard Agent Platform | 3,737.49 MB | Baseline (Standard Agent Stack) |

---

### 2. HTTP Gateway Latency and Throughput

![Latency Comparison](assets/latency_comparison.svg)

Multi-threaded concurrency sweep (concurrency=10, 500 requests per endpoint on localhost):

| Gateway Endpoint | Service Engine | Requests / Sec | p50 TTFB | p95 Latency | p99 Latency |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **cortex-rs `/api/cortex/get`** | Native Rust Axum | **2,056.8 req/s** | **3.56 ms** | 13.07 ms | 19.62 ms |
| **cortex-rs `/health`** | Native Rust Axum | **2,000.8 req/s** | **3.37 ms** | 13.65 ms | 19.59 ms |
| **spark-cockpit-rs `/api/pulse`** | Native Rust Axum | **1,921.7 req/s** | **4.30 ms** | 11.43 ms | 18.26 ms |
| **cortex-encoder-rs `/health`** | Native Rust Axum | **1,842.2 req/s** | **3.97 ms** | 13.97 ms | 20.14 ms |
| *Python FastAPI Baseline `/api/status`* | Python 3.12 / FastAPI | 214.5 req/s | 38.40 ms | 68.20 ms | 112.50 ms |

---

### 3. Local Neural Inference & SIMD Acceleration

Benchmarked on Grace Blackwell silicon:

| Workload | Model / Kernel | Execution Engine | p50 Latency | p95 Latency |
| :--- | :--- | :--- | :--- | :--- |
| **Short Query Vectorization** (8 tokens) | `BAAI/bge-base-en-v1.5` INT8 | ONNX Runtime C-API | **4.09 ms** | 5.81 ms |
| **Medium Context Vectorization** (32 tokens) | `BAAI/bge-base-en-v1.5` INT8 | ONNX Runtime C-API | **5.78 ms** | 6.00 ms |
| **Long Passage Vectorization** (128 tokens) | `BAAI/bge-base-en-v1.5` INT8 | ONNX Runtime C-API | **14.26 ms** | 29.91 ms |
| **SIMD Vector Reduction** | NuMojo / SIMD FFI | Compiled Mojo Native | **4.39 ms** | 4.82 ms |

---

## Universal Multi-Platform Portability

While our primary reference workstation is the NVIDIA DGX Spark (Grace Blackwell GB10), AIEN is architected from inception as a portable, hardware-independent stack. Every service relies on compiled Rust, standard C-ABI bindings, and the `spark-adapters` abstraction crate.

| Platform Target | Primary Accelerators | Engine & Execution Path | Deployment Status |
| :--- | :--- | :--- | :--- |
| **macOS (Apple Silicon)** | M1 / M2 / M3 / M4 (Pro / Max / Ultra) | Metal via MAX / llama.cpp, native aarch64 Rust | Verified & Supported |
| **Linux x86_64** | Intel / AMD CPUs, NVIDIA CUDA | glibc / musl native binaries, AVX-512 SIMD | Verified & Supported |
| **AMD ROCm** | Radeon RX 7000 / Instinct MI300 | ROCm / HIP targets, native Rust gateway | Verified & Supported |
| **NVIDIA DGX Spark** | Grace Blackwell GB10 / GB200 | Unified LPDDR5X, Modular MAX 26.5, NVFP4 | Reference Architecture |
| **Sovereign Bare Metal** | Air-gapped on-prem servers | Hardware TPM 2.0 vault, zero cloud calls | Verified & Supported |

To compile AIEN for your target architecture:

```bash
cargo build --release --workspace
```

---

## Reproducing the Benchmarks

To execute the benchmark suite locally:

```bash
# Clone the benchmarks repository
git clone https://github.com/aien-dev/benchmarks.git
cd benchmarks

# Run the test suite
cargo test --verbose

# Print the formatted terminal report
cargo run --release -- report

# Verify all metrics against invariant budgets
cargo run --release -- verify

# Re-render the SVG chart assets
cargo run --release -- generate --output assets
```

---

## Ecosystem Integration

- **Primary Repository**: [github.com/aien-dev/aien-dev](https://github.com/aien-dev/aien-dev)
- **Sovereign Core**: [github.com/aien-dev/aien-sovereign-core](https://github.com/aien-dev/aien-sovereign-core)
- **Website & Showcase**: [github.com/aien-dev/drakestapleton.com](https://github.com/aien-dev/drakestapleton.com) ([drakestapleton.com](https://drakestapleton.com))

---

## Governance & License

Licensed under the [Sovereign Resource Commons License 1.0 (SRCL-1.0)](LICENSE).
