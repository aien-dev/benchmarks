use std::time::Instant;

#[allow(dead_code)]
pub struct VectorBenchmarkResult {
    pub iterations: usize,
    pub dimensions: usize,
    pub elapsed_ms: f64,
    pub operations_per_sec: f64,
    pub p50_latency_us: f64,
}

pub fn run_vector_dot_product_benchmark(iterations: usize, dimensions: usize) -> VectorBenchmarkResult {
    let v1: Vec<f32> = (0..dimensions).map(|i| (i as f32) * 0.001).collect();
    let v2: Vec<f32> = (0..dimensions).map(|i| (i as f32) * 0.002).collect();

    let start = Instant::now();
    let mut accumulator = 0.0f32;

    for _ in 0..iterations {
        // Direct SIMD-friendly vector dot product
        let dot: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
        accumulator += dot;
    }

    let elapsed = start.elapsed();
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    let ops_per_sec = (iterations as f64) / elapsed.as_secs_f64().max(0.00001);
    let p50_us = (elapsed.as_micros() as f64) / (iterations as f64).max(1.0);

    // Prevent compiler optimization from dead-stripping computation
    if accumulator == 0.0000001 {
        println!("Edge case");
    }

    VectorBenchmarkResult {
        iterations,
        dimensions,
        elapsed_ms,
        operations_per_sec: ops_per_sec,
        p50_latency_us: p50_us,
    }
}
