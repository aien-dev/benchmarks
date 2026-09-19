use crate::models::NeuralMetric;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct StreamBenchmarkResult {
    pub ttft_ms: f64,
    pub itl_p50_ms: f64,
    #[allow(dead_code)]
    pub itl_p95_ms: f64,
    #[allow(dead_code)]
    pub total_tokens: usize,
    pub tokens_per_sec: f64,
}

pub fn measure_llm_streaming(
    host: &str,
    port: u16,
    model_name: &str,
    prompt: &str,
    max_tokens: usize,
) -> Result<StreamBenchmarkResult, String> {
    let addr = format!("{}:{}", host, port);
    let mut stream =
        TcpStream::connect(&addr).map_err(|e| format!("Connect to {} failed: {}", addr, e))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;

    let body = format!(
        r#"{{"model":"{}","messages":[{{"role":"user","content":"{}"}}],"max_tokens":{},"stream":true}}"#,
        model_name, prompt, max_tokens
    );

    let req = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        host, port, body.len(), body
    );

    let t0 = Instant::now();
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("Write error: {}", e))?;

    let mut buf = [0u8; 4096];
    let mut accumulated = Vec::new();
    let mut ttft_opt: Option<f64> = None;
    let mut itl_samples = Vec::new();
    let mut last_token_time = Instant::now();
    let mut total_tokens = 0;

    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let now = Instant::now();
        accumulated.extend_from_slice(&buf[..n]);

        while let Some(pos) = accumulated.windows(2).position(|w| w == b"\n\n") {
            let part = accumulated.drain(..pos + 2).collect::<Vec<u8>>();
            let text = String::from_utf8_lossy(&part);
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("data: ") && !trimmed.starts_with("data: [DONE]") {
                    total_tokens += 1;
                    if ttft_opt.is_none() {
                        ttft_opt = Some(t0.elapsed().as_secs_f64() * 1000.0);
                        last_token_time = now;
                    } else {
                        let itl = now.duration_since(last_token_time).as_secs_f64() * 1000.0;
                        itl_samples.push(itl);
                        last_token_time = now;
                    }
                }
            }
        }
    }

    let ttft = ttft_opt.ok_or_else(|| "No tokens received from LLM endpoint".to_string())?;
    let total_elapsed = t0.elapsed().as_secs_f64();
    let tokens_per_sec = if total_elapsed > 0.0 {
        total_tokens as f64 / total_elapsed
    } else {
        0.0
    };

    itl_samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let itl_p50 = if itl_samples.is_empty() {
        0.0
    } else {
        itl_samples[itl_samples.len() * 50 / 100]
    };
    let itl_p95 = if itl_samples.is_empty() {
        0.0
    } else {
        itl_samples[itl_samples.len() * 95 / 100]
    };

    Ok(StreamBenchmarkResult {
        ttft_ms: ttft,
        itl_p50_ms: itl_p50,
        itl_p95_ms: itl_p95,
        total_tokens,
        tokens_per_sec,
    })
}

pub fn measure_embedding_latency(
    host: &str,
    port: u16,
    text: &str,
    iterations: usize,
) -> Result<(f64, f64), String> {
    let mut latencies = Vec::with_capacity(iterations);
    let body = format!(r#"{{"text":"{}"}}"#, text);

    for _ in 0..iterations {
        let addr = format!("{}:{}", host, port);
        let mut stream =
            TcpStream::connect(&addr).map_err(|e| format!("Connect to {} failed: {}", addr, e))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;

        let req = format!(
            "POST /embed HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            host, port, body.len(), body
        );

        let t0 = Instant::now();
        stream
            .write_all(req.as_bytes())
            .map_err(|e| format!("Write error: {}", e))?;
        let mut buf = [0u8; 1024];
        let mut read_any = false;
        while let Ok(n) = stream.read(&mut buf) {
            if n == 0 {
                break;
            }
            read_any = true;
        }
        if read_any {
            latencies.push(t0.elapsed().as_secs_f64() * 1000.0);
        }
    }

    if latencies.is_empty() {
        return Err("No successful embedding responses".to_string());
    }

    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p50 = latencies[latencies.len() * 50 / 100];
    let p95 = latencies[latencies.len() * 95 / 100];
    Ok((p50, p95))
}

pub fn run_neural_inference_benchmarks() -> Vec<NeuralMetric> {
    let mut metrics = Vec::new();

    // 1. Benchmark live primary LLM seat on port 18006 (Modular MAX on GB10 GPU)
    println!("        Benchmarking live Modular MAX seat (port 18006, atlas-lightning-omni)...");
    let _ = measure_llm_streaming("127.0.0.1", 18006, "atlas-lightning-omni", "Warmup", 4);
    match measure_llm_streaming(
        "127.0.0.1",
        18006,
        "atlas-lightning-omni",
        "Explain unified memory architecture in ten words.",
        20,
    ) {
        Ok(res) => {
            println!(
                "        Modular MAX (GB10 GPU): TTFT {:.2} ms | ITL p50 {:.2} ms | Tok/s {:.1}",
                res.ttft_ms, res.itl_p50_ms, res.tokens_per_sec
            );
            metrics.push(NeuralMetric {
                workload: "First Token Latency (TTFT) - GPU Seat".to_string(),
                model: "Nemotron 3.5 Lightning 30B (BF16)".to_string(),
                engine: "Modular MAX (Grace Blackwell GB10 GPU)".to_string(),
                p50_ms: res.ttft_ms,
                p95_ms: res.ttft_ms,
            });
            metrics.push(NeuralMetric {
                workload: "Inter-Token Latency (ITL) - GPU Seat".to_string(),
                model: "Nemotron 3.5 Lightning 30B (BF16)".to_string(),
                engine: "Modular MAX (Grace Blackwell GB10 GPU)".to_string(),
                p50_ms: res.itl_p50_ms,
                p95_ms: res.itl_p95_ms,
            });
        }
        Err(e) => {
            println!("        [Notice] Modular MAX GPU seat (18006) skipped: {}", e);
        }
    }

    // 2. Benchmark live fallback LLM seat on port 18082 (Modular MAX CPU)
    println!("        Benchmarking live fallback seat (port 18082, unsloth/Llama-3.2-1B-Instruct)...");
    let _ = measure_llm_streaming(
        "127.0.0.1",
        18082,
        "unsloth/Llama-3.2-1B-Instruct",
        "Warmup",
        4,
    );
    match measure_llm_streaming(
        "127.0.0.1",
        18082,
        "unsloth/Llama-3.2-1B-Instruct",
        "Explain unified memory architecture in ten words.",
        20,
    ) {
        Ok(res) => {
            println!(
                "        Modular MAX (CPU): TTFT {:.2} ms | ITL p50 {:.2} ms | Tok/s {:.1}",
                res.ttft_ms, res.itl_p50_ms, res.tokens_per_sec
            );
            metrics.push(NeuralMetric {
                workload: "First Token Latency (TTFT) - CPU Fallback".to_string(),
                model: "Llama 3.2 1B Instruct (Q4_K)".to_string(),
                engine: "Modular MAX (Grace Neoverse CPU)".to_string(),
                p50_ms: res.ttft_ms,
                p95_ms: res.ttft_ms,
            });
            metrics.push(NeuralMetric {
                workload: "Inter-Token Latency (ITL) - CPU Fallback".to_string(),
                model: "Llama 3.2 1B Instruct (Q4_K)".to_string(),
                engine: "Modular MAX (Grace Neoverse CPU)".to_string(),
                p50_ms: res.itl_p50_ms,
                p95_ms: res.itl_p95_ms,
            });
        }
        Err(e) => {
            println!("        [Notice] Modular MAX CPU seat (18082) skipped: {}", e);
        }
    }

    // 3. Benchmark live embedding service on port 18081 (cortex-encoder-rs ONNX)
    println!("        Benchmarking live embedding service (port 18081, BGE-base-en-v1.5 INT8)...");
    match measure_embedding_latency(
        "127.0.0.1",
        18081,
        "The sovereign architecture achieves deterministic low-latency execution on Grace Blackwell.",
        10,
    ) {
        Ok((p50, p95)) => {
            println!(
                "        cortex-encoder-rs (ONNX INT8): p50 {:.2} ms | p95 {:.2} ms",
                p50, p95
            );
            metrics.push(NeuralMetric {
                workload: "Bi-Encoder Vector Embedding".to_string(),
                model: "BAAI/bge-base-en-v1.5 (INT8)".to_string(),
                engine: "cortex-encoder-rs (ONNX Runtime)".to_string(),
                p50_ms: p50,
                p95_ms: p95,
            });
        }
        Err(e) => {
            println!("        [Notice] cortex-encoder-rs (18081) skipped: {}", e);
        }
    }

    metrics
}
