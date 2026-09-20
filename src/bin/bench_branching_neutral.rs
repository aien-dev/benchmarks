//! Neutral out-of-process benchmark runner for agentic branching suites on NVIDIA DGX Spark GB10.
//! Supports simultaneous branch dispatch against AIEN, Modular MAX, and vLLM.

use clap::Parser;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Barrier;

#[derive(Parser, Debug)]
#[command(name = "bench_branching_neutral")]
#[command(about = "Neutral Out-of-Process Benchmark Runner for Real-Model Agentic Branching")]
struct Args {
    /// Path to the frozen benchmark manifest JSON
    #[arg(short, long, default_value = "manifests/qwen25_7b_branching_32k.json")]
    manifest: PathBuf,

    /// Comma-separated list of target engines: aien, max, vllm
    #[arg(short, long, default_value = "aien,max,vllm")]
    engines: String,

    /// Comma-separated list of branch counts to sweep: 1,10,25,50,100,250,500
    #[arg(short, long, default_value = "1,10,25,50,100,250,500")]
    branches: String,

    /// Repetitions per branch count
    #[arg(short, long, default_value_t = 3)]
    repetitions: usize,

    /// Maximum tokens to generate per branch
    #[arg(long, default_value_t = 256)]
    max_tokens: usize,

    /// Output directory for trial JSONL and summary results
    #[arg(short, long, default_value = "data/branching_results")]
    output_dir: PathBuf,

    /// AIEN base URL
    #[arg(long, default_value = "http://127.0.0.1:18092/v1")]
    aien_url: String,

    /// Modular MAX base URL
    #[arg(long, default_value = "http://127.0.0.1:18098/v1")]
    max_url: String,

    /// vLLM base URL
    #[arg(long, default_value = "http://127.0.0.1:18094/v1")]
    vllm_url: String,

    /// Optional PID of the active engine to monitor smaps_rollup
    #[arg(long)]
    engine_pid: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    model_id: String,
    prefix_token_count: usize,
    prefix_token_hash: String,
    prefix_token_ids: Vec<u32>,
    branches: Vec<BranchSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct BranchSpec {
    branch_id: usize,
    category: String,
    text: String,
    token_ids: Vec<u32>,
    token_count: usize,
}

#[derive(Debug, Clone, Serialize)]
struct BranchTrialResult {
    branch_id: usize,
    category: String,
    ttft_ms: f64,
    total_time_ms: f64,
    tokens_generated: usize,
    tok_per_sec: f64,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TrialSummary {
    engine: String,
    branch_count: usize,
    repetition: usize,
    completed_branches: usize,
    failed_branches: usize,
    ttft_p50_ms: f64,
    ttft_p95_ms: f64,
    ttft_p99_ms: f64,
    aggregate_tokens_per_sec: f64,
    total_wall_time_ms: f64,
    pss_mb_before: Option<f64>,
    pss_mb_peak: Option<f64>,
}

fn calculate_percentile(values: &mut [f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() as f64 - 1.0) * pct).round() as usize;
    values[idx.min(values.len() - 1)]
}

async fn execute_single_branch(
    client: Client,
    url: String,
    model: String,
    branch: BranchSpec,
    max_tokens: usize,
    barrier: Arc<Barrier>,
) -> BranchTrialResult {
    barrier.wait().await;

    let start_time = Instant::now();
    let payload = serde_json::json!({
        "model": model,
        "prompt": branch.text,
        "max_tokens": max_tokens,
        "temperature": 0.0,
        "stream": true,
    });

    let req = client
        .post(format!("{}/completions", url.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .json(&payload);

    let res = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return BranchTrialResult {
                branch_id: branch.branch_id,
                category: branch.category,
                ttft_ms: 0.0,
                total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                tokens_generated: 0,
                tok_per_sec: 0.0,
                success: false,
                error: Some(e.to_string()),
            };
        }
    };

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return BranchTrialResult {
            branch_id: branch.branch_id,
            category: branch.category,
            ttft_ms: 0.0,
            total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
            tokens_generated: 0,
            tok_per_sec: 0.0,
            success: false,
            error: Some(format!("HTTP {}: {}", status, body)),
        };
    }

    let mut stream = res.bytes_stream();
    let mut first_token_time: Option<Instant> = None;
    let mut tokens_count = 0;

    while let Some(item) = stream.next().await {
        match item {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    if line.starts_with("data:") {
                        let data = line.trim_start_matches("data:").trim();
                        if data == "[DONE]" {
                            break;
                        }
                        if first_token_time.is_none() {
                            first_token_time = Some(Instant::now());
                        }
                        tokens_count += 1;
                    }
                }
            }
            Err(e) => {
                return BranchTrialResult {
                    branch_id: branch.branch_id,
                    category: branch.category,
                    ttft_ms: first_token_time
                        .map(|t| (t - start_time).as_secs_f64() * 1000.0)
                        .unwrap_or(0.0),
                    total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                    tokens_generated: tokens_count,
                    tok_per_sec: 0.0,
                    success: false,
                    error: Some(e.to_string()),
                };
            }
        }
    }

    let total_time_ms = start_time.elapsed().as_secs_f64() * 1000.0;
    let ttft_ms = first_token_time
        .map(|t| (t - start_time).as_secs_f64() * 1000.0)
        .unwrap_or(total_time_ms);
    let tok_per_sec = if total_time_ms > 0.0 {
        (tokens_count as f64) / (total_time_ms / 1000.0)
    } else {
        0.0
    };

    BranchTrialResult {
        branch_id: branch.branch_id,
        category: branch.category,
        ttft_ms,
        total_time_ms,
        tokens_generated: tokens_count,
        tok_per_sec,
        success: true,
        error: None,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    fs::create_dir_all(&args.output_dir)?;

    println!("Loading manifest from {}...", args.manifest.display());
    let manifest_str = fs::read_to_string(&args.manifest)?;
    let manifest: Manifest = serde_json::from_str(&manifest_str)?;
    println!(
        "Manifest loaded: model={}, prefix_tokens={}, total_available_branches={}",
        manifest.model_id,
        manifest.prefix_token_count,
        manifest.branches.len()
    );

    let branch_counts: Vec<usize> = args
        .branches
        .split(',')
        .filter_map(|s| s.trim().parse::<usize>().ok())
        .collect();

    let engines: Vec<String> = args
        .engines
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let mut all_summaries = Vec::new();
    let client = Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;

    for engine in &engines {
        let engine_url = match engine.as_str() {
            "aien" => args.aien_url.clone(),
            "max" => args.max_url.clone(),
            "vllm" => args.vllm_url.clone(),
            other => {
                eprintln!("Unknown engine {}, skipping", other);
                continue;
            }
        };

        for &n_branches in &branch_counts {
            if n_branches > manifest.branches.len() {
                eprintln!(
                    "Requested branches {} exceeds manifest branches {}, capping",
                    n_branches,
                    manifest.branches.len()
                );
                continue;
            }

            for rep in 1..=args.repetitions {
                println!(
                    "--- Running Engine: {}, Branches: {}, Repetition: {}/{} ---",
                    engine, n_branches, rep, args.repetitions
                );

                let pss_before = args
                    .engine_pid
                    .and_then(aien_benchmarks::harness::smaps::read_smaps_rollup)
                    .map(|s| s.pss_mb());

                let barrier = Arc::new(Barrier::new(n_branches));
                let mut tasks = Vec::with_capacity(n_branches);

                let trial_start = Instant::now();
                for i in 0..n_branches {
                    let branch = manifest.branches[i].clone();
                    let b_client = client.clone();
                    let b_url = engine_url.clone();
                    let b_model = manifest.model_id.clone();
                    let b_barrier = barrier.clone();
                    let max_tok = args.max_tokens;

                    tasks.push(tokio::spawn(async move {
                        execute_single_branch(
                            b_client,
                            b_url,
                            b_model,
                            branch,
                            max_tok,
                            b_barrier,
                        )
                        .await
                    }));
                }

                let mut branch_results = Vec::with_capacity(n_branches);
                for task in tasks {
                    match task.await {
                        Ok(res) => branch_results.push(res),
                        Err(e) => eprintln!("Task join error: {}", e),
                    }
                }

                let wall_time_ms = trial_start.elapsed().as_secs_f64() * 1000.0;
                let pss_peak = args
                    .engine_pid
                    .and_then(aien_benchmarks::harness::smaps::read_smaps_rollup)
                    .map(|s| s.pss_mb());

                let trial_filename = format!(
                    "trial-{}-{}-rep{}.jsonl",
                    engine, n_branches, rep
                );
                let trial_path = args.output_dir.join(&trial_filename);
                let mut file = File::create(&trial_path)?;
                for b in &branch_results {
                    writeln!(file, "{}", serde_json::to_string(b)?)?;
                }

                let completed = branch_results.iter().filter(|b| b.success).count();
                let failed = branch_results.len() - completed;

                let mut ttft_vec: Vec<f64> = branch_results
                    .iter()
                    .filter(|b| b.success)
                    .map(|b| b.ttft_ms)
                    .collect();

                let total_tokens: usize = branch_results
                    .iter()
                    .filter(|b| b.success)
                    .map(|b| b.tokens_generated)
                    .sum();

                let agg_tok_per_sec = if wall_time_ms > 0.0 {
                    (total_tokens as f64) / (wall_time_ms / 1000.0)
                } else {
                    0.0
                };

                let summary = TrialSummary {
                    engine: engine.clone(),
                    branch_count: n_branches,
                    repetition: rep,
                    completed_branches: completed,
                    failed_branches: failed,
                    ttft_p50_ms: calculate_percentile(&mut ttft_vec, 0.50),
                    ttft_p95_ms: calculate_percentile(&mut ttft_vec, 0.95),
                    ttft_p99_ms: calculate_percentile(&mut ttft_vec, 0.99),
                    aggregate_tokens_per_sec: agg_tok_per_sec,
                    total_wall_time_ms: wall_time_ms,
                    pss_mb_before: pss_before,
                    pss_mb_peak: pss_peak,
                };

                println!(
                    "  -> Completed: {}/{}, p95 TTFT: {:.2}ms, Agg Throughput: {:.2} tok/s, Wall Time: {:.2}s",
                    summary.completed_branches,
                    n_branches,
                    summary.ttft_p95_ms,
                    summary.aggregate_tokens_per_sec,
                    summary.total_wall_time_ms / 1000.0
                );

                all_summaries.push(summary);
            }
        }
    }

    let summary_path = args.output_dir.join("summary.json");
    let summary_file = File::create(&summary_path)?;
    serde_json::to_writer_pretty(summary_file, &all_summaries)?;
    println!("Benchmark completed. Summary written to {}", summary_path.display());

    Ok(())
}
