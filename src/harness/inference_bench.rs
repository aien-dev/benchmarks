use super::llm_stream::{self, distribution, Distribution, StreamConfig, TokenAccounting};
use crate::models::NeuralMetric;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub fn measure_embedding_latency(
    host: &str,
    port: u16,
    text: &str,
    iterations: usize,
) -> Result<Distribution, String> {
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

    Ok(distribution(&latencies))
}

/// Live neural rows for the measurement sweep: the configured streaming endpoint (if any)
/// and the embedding service. Labels come from the configuration, never from constants.
pub fn run_neural_inference_benchmarks(llm: Option<&StreamConfig>) -> Vec<NeuralMetric> {
    let mut metrics = Vec::new();

    if let Some(cfg) = llm {
        println!(
            "        Benchmarking streaming endpoint {} (model {}, engine {}, {} request(s), concurrency {})...",
            cfg.endpoint(),
            cfg.model,
            cfg.engine,
            cfg.requests,
            cfg.concurrency
        );
        let run = llm_stream::run_stream_benchmark(cfg);
        let s = llm_stream::summarize(&run.samples, run.wall);
        print_stream_summary(&s);
        if s.requests_ok > 0 {
            metrics.push(NeuralMetric {
                measurement_id: Some("BENCH-LIVE-TTFT-001".to_string()),
                workload: format!("First content event latency (TTFT), N={}", s.ttft.samples),
                model: cfg.model.clone(),
                engine: cfg.engine.clone(),
                p50_ms: s.ttft.p50.unwrap_or_default(),
                p95_ms: s.ttft.p95,
            });
            if let Some(p50) = s.inter_event.p50 {
                metrics.push(NeuralMetric {
                    measurement_id: Some("BENCH-LIVE-INTER-EVENT-001".to_string()),
                    workload: format!(
                        "Inter-event latency (SSE events, not tokens), N={}",
                        s.inter_event.samples
                    ),
                    model: cfg.model.clone(),
                    engine: cfg.engine.clone(),
                    p50_ms: p50,
                    p95_ms: s.inter_event.p95,
                });
            }
            if let Some(tpot) = s.tpot.as_ref().filter(|d| d.p50.is_some()) {
                metrics.push(NeuralMetric {
                    measurement_id: Some("BENCH-LIVE-TPOT-001".to_string()),
                    workload: format!("Time per output token (backend usage), N={}", tpot.samples),
                    model: cfg.model.clone(),
                    engine: cfg.engine.clone(),
                    p50_ms: tpot.p50.unwrap_or_default(),
                    p95_ms: tpot.p95,
                });
            }
        }
    }

    // Embedding service on port 18081 (cortex-encoder-rs ONNX); unchanged target.
    println!("        Benchmarking live embedding service (port 18081, BGE-base-en-v1.5 INT8)...");
    match measure_embedding_latency(
        "127.0.0.1",
        18081,
        "The sovereign architecture achieves deterministic low-latency execution on Grace Blackwell.",
        10,
    ) {
        Ok(d) => {
            println!(
                "        cortex-encoder-rs (ONNX INT8): p50 {:.2} ms | p95 {} | N={}",
                d.p50.unwrap_or_default(),
                fmt_opt_ms(d.p95),
                d.samples
            );
            metrics.push(NeuralMetric {
                measurement_id: Some("BENCH-LIVE-EMBED-001".to_string()),
                workload: format!("Bi-Encoder Vector Embedding, N={}", d.samples),
                model: "BAAI/bge-base-en-v1.5 (INT8)".to_string(),
                engine: "cortex-encoder-rs (ONNX Runtime)".to_string(),
                p50_ms: d.p50.unwrap_or_default(),
                p95_ms: d.p95,
            });
        }
        Err(e) => {
            println!("        [Notice] cortex-encoder-rs (18081) skipped: {}", e);
        }
    }

    metrics
}

pub fn fmt_opt_ms(v: Option<f64>) -> String {
    v.map(|x| format!("{:.2} ms", x))
        .unwrap_or_else(|| "n/a (too few samples)".to_string())
}

/// Terminal summary; under events_not_tokens it never prints tokens per second.
pub fn print_stream_summary(s: &llm_stream::StreamSummary) {
    println!(
        "        requests ok {}/{} | wall {:.1} ms | token accounting: {}",
        s.requests_ok,
        s.requests_attempted,
        s.wall_ms,
        s.token_accounting.as_str()
    );
    for (kind, n) in &s.failures {
        println!("        failed: {} x {}", n, kind);
    }
    if s.requests_ok == 0 {
        return;
    }
    println!(
        "        TTFT p50 {} | p95 {} | N={}",
        fmt_opt_ms(s.ttft.p50),
        fmt_opt_ms(s.ttft.p95),
        s.ttft.samples
    );
    println!(
        "        inter-event (events, not tokens) p50 {} | p95 {} | N={} | coalesced in one read: {}",
        fmt_opt_ms(s.inter_event.p50),
        fmt_opt_ms(s.inter_event.p95),
        s.inter_event.samples,
        s.events_coalesced_in_read
    );
    if let Some(eps) = s.events_per_sec {
        println!("        content events/s: {:.1}", eps);
    }
    if s.token_accounting == TokenAccounting::BackendUsage {
        if let (Some(total), Some(tps)) = (s.output_tokens_total, s.output_tokens_per_sec) {
            println!(
                "        backend usage: {} output tokens, {:.1} output tokens/s",
                total, tps
            );
        }
        if let Some(t) = &s.tpot {
            println!(
                "        time per output token p50 {} | p95 {} | N={}",
                fmt_opt_ms(t.p50),
                fmt_opt_ms(t.p95),
                t.samples
            );
        }
    }
}
