use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkData {
    pub benchmark_suite: String,
    pub version: String,
    pub timestamp: String,
    pub hardware: HardwareInfo,
    pub memory_rss: Vec<MemoryMetric>,
    pub latency_concurrency: Vec<LatencyMetric>,
    pub neural_inference: Vec<NeuralMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub system: String,
    pub processor: String,
    pub architecture: String,
    pub memory_unified_gb: u32,
    pub os: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetric {
    pub service: String,
    pub role: String,
    pub architecture: String,
    pub rss_mb: f64,
    pub baseline_rss_mb: f64,
    pub reduction_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NeuralMetric {
    pub workload: String,
    pub model: String,
    pub engine: String,
    pub p50_ms: f64,
    pub p95_ms: f64,
}
