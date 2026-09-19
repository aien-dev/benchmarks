use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkData {
    pub benchmark_suite: String,
    pub version: String,
    pub timestamp: String,
    pub hardware: HardwareInfo,
    #[serde(default)]
    pub environment: EnvironmentInfo,
    pub memory_rss: Vec<MemoryMetric>,
    pub latency_concurrency: Vec<LatencyMetric>,
    #[serde(default)]
    pub like_for_like: Vec<LikeForLikeMetric>,
    pub neural_inference: Vec<NeuralMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardwareInfo {
    pub system: String,
    pub processor: String,
    pub architecture: String,
    pub memory_unified_gb: u32,
    pub os: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnvironmentInfo {
    pub rustc_version: String,
    pub python_version: String,
    pub kernel_version: String,
    pub page_size_kb: u32,
    pub concurrency_tested: u32,
    pub requests_per_endpoint: u32,
    pub warmup_requests: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemoryMetric {
    pub service: String,
    pub role: String,
    pub architecture: String,
    pub rss_mb: f64,
    pub baseline_rss_mb: f64,
    pub reduction_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LatencyMetric {
    pub service: String,
    pub endpoint: String,
    pub description: String,
    pub engine: String,
    pub requests_per_sec: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub concurrency: u32,
    pub sample_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LikeForLikeMetric {
    pub category: String,
    pub workload: String,
    pub rust_engine: String,
    pub rust_p50_ms: f64,
    pub rust_throughput_req_s: f64,
    pub rust_rss_mb: f64,
    pub python_engine: String,
    pub python_p50_ms: f64,
    pub python_throughput_req_s: f64,
    pub python_rss_mb: f64,
    pub speedup_factor: f64,
    pub memory_reduction_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NeuralMetric {
    pub workload: String,
    pub model: String,
    pub engine: String,
    pub p50_ms: f64,
    pub p95_ms: f64,
}
