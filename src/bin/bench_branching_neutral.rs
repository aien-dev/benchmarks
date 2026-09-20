//! Neutral out-of-process benchmark runner for agentic branching suites on NVIDIA DGX Spark GB10.
//! Supports simultaneous branch dispatch against AIEN, Modular MAX, and vLLM.
//! Sends complete 32K shared prefix + branch suffix tokens, performs continuous 50ms PSS/RSS
//! sampling during execution, validates model revision, and records exact token-level ITL distributions.

use clap::Parser;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

    /// Require exact model ID check against /v1/models
    #[arg(long, default_value_t = true)]
    verify_model_id: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    model_id: String,
    model_revision: String,
    prefix_token_count: usize,
    prefix_token_hash: String,
    prefix_token_ids: Vec<u32>,
    prefix_text: Option<String>,
    branches: Vec<BranchSpec>,
}

#[derive(Debug, Clone, Deserialize)]
struct BranchSpec {
    branch_id: usize,
    category: String,
    text: String,
    token_ids: Vec<u32>,
    token_count: usize,
    #[serde(default)]
    expected_first_token: Option<u32>,
    #[serde(default)]
    expected_first_16_tokens: Option<Vec<u32>>,
    #[serde(default)]
    expected_full_tokens: Option<Vec<u32>>,
    #[serde(default)]
    expected_output_tokens: Option<Vec<u32>>,
    #[serde(default)]
    expected_output_hash: Option<String>,
    #[serde(default)]
    expected_output_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct BranchTrialResult {
    branch_id: usize,
    category: String,
    input_tokens: usize,
    ttft_ms: f64,
    itl_p50_ms: f64,
    itl_p95_ms: f64,
    itl_p99_ms: f64,
    total_time_ms: f64,
    tokens_generated: usize,
    tok_per_sec: f64,
    success: bool,
    oracle_match: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TrialSummary {
    engine: String,
    branch_count: usize,
    repetition: usize,
    completed_branches: usize,
    failed_branches: usize,
    prefix_tokens: usize,
    ttft_p50_ms: f64,
    ttft_p95_ms: f64,
    ttft_p99_ms: f64,
    itl_p50_ms: f64,
    itl_p95_ms: f64,
    aggregate_tokens_per_sec: f64,
    total_wall_time_ms: f64,
    pss_mb_before: Option<f64>,
    pss_mb_peak: Option<f64>,
    rss_mb_peak: Option<f64>,
    min_mem_available_mb: Option<f64>,
}

fn hash_tokens(token_ids: &[u32]) -> String {
    let mut hasher = Sha256::new();
    for &tid in token_ids {
        hasher.update(tid.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn calculate_percentile(values: &mut [f64], pct: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() as f64 - 1.0) * pct).round() as usize;
    values[idx.min(values.len() - 1)]
}

fn read_system_mem_available_kb() -> Option<u64> {
    let content = fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            return rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok());
        }
    }
    None
}

async fn verify_engine_model(
    client: &Client,
    url: &str,
    expected_model: &str,
    expected_revision: &str,
) -> Result<(), String> {
    let endpoint = format!("{}/models", url.trim_end_matches("/"));
    let res = client
        .get(&endpoint)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("Failed to reach engine /v1/models at {}: {}", endpoint, e))?;

    if !res.status().is_success() {
        return Err(format!("Engine returned HTTP {} on /v1/models", res.status()));
    }

    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("Failed to parse /v1/models JSON: {}", e))?;

    let mut found = false;
    let mut rev_matched = false;
    let rev_short = &expected_revision[..expected_revision.len().min(8)];

    if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
        for m in data {
            if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                if id.contains(expected_model) || expected_model.contains(id) {
                    found = true;
                    if let Some(root) = m.get("root").and_then(|r| r.as_str()) {
                        if root.contains(rev_short) || root.contains(expected_revision) {
                            rev_matched = true;
                        }
                    }
                    if let Some(rev) = m.get("revision").and_then(|r| r.as_str()) {
                        if rev.contains(rev_short) || rev.contains(expected_revision) {
                            rev_matched = true;
                        }
                    }
                    if id.contains(rev_short) || id.contains(expected_revision) {
                        rev_matched = true;
                    }
                    break;
                }
            }
        }
    }

    if found && rev_matched {
        Ok(())
    } else if found {
        Err(format!(
            "Model {} found at {}, but pinned revision {} could not be verified (server response did not contain matching revision or path)",
            expected_model, endpoint, expected_revision
        ))
    } else {
        Err(format!(
            "Model {} not found in active engine models list at {}",
            expected_model, endpoint
        ))
    }
}

async fn execute_single_branch(
    client: Client,
    url: String,
    model: String,
    prefix_tokens: Arc<Vec<u32>>,
    branch: BranchSpec,
    max_tokens: usize,
    barrier: Arc<Barrier>,
) -> BranchTrialResult {
    // 1. Construct complete 32K+ input token prompt
    let mut full_token_ids = Vec::with_capacity(prefix_tokens.len() + branch.token_ids.len());
    full_token_ids.extend_from_slice(&prefix_tokens);
    full_token_ids.extend_from_slice(&branch.token_ids);
    let input_tokens = full_token_ids.len();

    // 2. Synchronize all concurrent branches before firing
    barrier.wait().await;

    let start_time = Instant::now();
    let payload = serde_json::json!({
        "model": model,
        "prompt": full_token_ids,
        "max_tokens": max_tokens,
        "temperature": 0.0,
        "stream": true,
    });

    let req = client
        .post(format!("{}/completions", url.trim_end_matches("/")))
        .header("Content-Type", "application/json")
        .json(&payload);

    let res = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return BranchTrialResult {
                branch_id: branch.branch_id,
                category: branch.category,
                input_tokens,
                ttft_ms: 0.0,
                itl_p50_ms: 0.0,
                itl_p95_ms: 0.0,
                itl_p99_ms: 0.0,
                total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                tokens_generated: 0,
                tok_per_sec: 0.0,
                success: false,
                oracle_match: None,
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
            input_tokens,
            ttft_ms: 0.0,
            itl_p50_ms: 0.0,
            itl_p95_ms: 0.0,
            itl_p99_ms: 0.0,
            total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
            tokens_generated: 0,
            tok_per_sec: 0.0,
            success: false,
            oracle_match: None,
            error: Some(format!("HTTP {}: {}", status, body)),
        };
    }

    let mut stream = res.bytes_stream();
    let mut first_token_time: Option<Instant> = None;
    let mut last_token_time: Option<Instant> = None;
    let mut itls: Vec<f64> = Vec::new();
    let mut tokens_count = 0;
    let mut generated_text = String::new();
    let mut generated_token_ids: Vec<u32> = Vec::new();
    let mut line_buffer = String::new();

    while let Some(item) = stream.next().await {
        match item {
            Ok(bytes) => {
                line_buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(newline_pos) = line_buffer.find('\n') {
                    let line = line_buffer[..newline_pos].trim().to_string();
                    line_buffer.drain(..=newline_pos);

                    if line.starts_with("data:") {
                        let data = line.trim_start_matches("data:").trim();
                        if data == "[DONE]" {
                            break;
                        }
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                            let now = Instant::now();
                            if first_token_time.is_none() {
                                first_token_time = Some(now);
                            } else if let Some(prev) = last_token_time {
                                let itl = (now - prev).as_secs_f64() * 1000.0;
                                itls.push(itl);
                            }
                            last_token_time = Some(now);

                            if let Some(choices) = val.get("choices").and_then(|c| c.as_array()) {
                                if let Some(choice) = choices.first() {
                                    if let Some(text_delta) = choice.get("text").and_then(|t| t.as_str()) {
                                        generated_text.push_str(text_delta);
                                    }
                                    if let Some(tid) = choice.get("token_id").and_then(|t| t.as_u64()) {
                                        generated_token_ids.push(tid as u32);
                                        tokens_count += 1;
                                    } else if let Some(tids) = choice.get("token_ids").and_then(|t| t.as_array()) {
                                        for t in tids {
                                            if let Some(id) = t.as_u64() {
                                                generated_token_ids.push(id as u32);
                                                tokens_count += 1;
                                            }
                                        }
                                    } else {
                                        tokens_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                return BranchTrialResult {
                    branch_id: branch.branch_id,
                    category: branch.category,
                    input_tokens,
                    ttft_ms: first_token_time
                        .map(|t| (t - start_time).as_secs_f64() * 1000.0)
                        .unwrap_or(0.0),
                    itl_p50_ms: calculate_percentile(&mut itls, 0.50),
                    itl_p95_ms: calculate_percentile(&mut itls, 0.95),
                    itl_p99_ms: calculate_percentile(&mut itls, 0.99),
                    total_time_ms: start_time.elapsed().as_secs_f64() * 1000.0,
                    tokens_generated: tokens_count,
                    tok_per_sec: 0.0,
                    success: false,
                    oracle_match: None,
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

    let oracle_match = if let Some(ref exp) = branch
        .expected_output_tokens
        .as_ref()
        .or(branch.expected_full_tokens.as_ref())
    {
        if !exp.is_empty() && !generated_token_ids.is_empty() {
            Some(&generated_token_ids == *exp)
        } else {
            None
        }
    } else if let Some(ref exp_16) = branch.expected_first_16_tokens {
        if !exp_16.is_empty() && generated_token_ids.len() >= exp_16.len() {
            Some(&generated_token_ids[..exp_16.len()] == &exp_16[..])
        } else {
            Some(false)
        }
    } else if let Some(ref exp_hash) = branch.expected_output_hash {
        if !exp_hash.is_empty() && !generated_token_ids.is_empty() {
            Some(hash_tokens(&generated_token_ids) == *exp_hash)
        } else {
            None
        }
    } else if let Some(exp_first) = branch.expected_first_token {
        if !generated_token_ids.is_empty() {
            Some(generated_token_ids[0] == exp_first)
        } else {
            Some(false)
        }
    } else if let Some(ref exp_text) = branch.expected_output_text {
        if !exp_text.is_empty() && !generated_text.is_empty() {
            Some(generated_text.trim() == exp_text.trim())
        } else {
            None
        }
    } else {
        None
    };

    BranchTrialResult {
        branch_id: branch.branch_id,
        category: branch.category,
        input_tokens,
        ttft_ms,
        itl_p50_ms: calculate_percentile(&mut itls, 0.50),
        itl_p95_ms: calculate_percentile(&mut itls, 0.95),
        itl_p99_ms: calculate_percentile(&mut itls, 0.99),
        total_time_ms,
        tokens_generated: tokens_count,
        tok_per_sec,
        success: true,
        oracle_match,
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
        "Manifest loaded: model={}, rev={}, prefix_tokens={}, available_branches={}",
        manifest.model_id,
        &manifest.model_revision[..8],
        manifest.prefix_token_count,
        manifest.branches.len()
    );

    let prefix_tokens = Arc::new(manifest.prefix_token_ids.clone());

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
        .timeout(Duration::from_secs(600))
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

        if args.verify_model_id {
            print!(
                "Verifying model {} (rev {}) on engine {} at {}... ",
                manifest.model_id,
                &manifest.model_revision[..8],
                engine,
                engine_url
            );
            match verify_engine_model(&client, &engine_url, &manifest.model_id, &manifest.model_revision).await {
                Ok(_) => println!("verified."),
                Err(e) => {
                    eprintln!("FAILED: {}", e);
                    eprintln!("Skipping engine {} due to model verification failure.", engine);
                    continue;
                }
            }
        }

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

                let pss_mb_before = args
                    .engine_pid
                    .and_then(aien_benchmarks::harness::smaps::read_process_tree_smaps)
                    .map(|s| s.pss_mb());

                // Start continuous 50ms memory sampler
                let stop_monitor = Arc::new(AtomicBool::new(false));
                let mon_stop = stop_monitor.clone();
                let mon_pid = args.engine_pid;

                let monitor_task = tokio::spawn(async move {
                    let mut peak_pss: f64 = 0.0;
                    let mut peak_rss: f64 = 0.0;
                    let mut min_avail_kb: u64 = u64::MAX;

                    while !mon_stop.load(Ordering::Relaxed) {
                        if let Some(p) = mon_pid {
                            if let Some(snap) = aien_benchmarks::harness::smaps::read_process_tree_smaps(p) {
                                if snap.pss_mb() > peak_pss {
                                    peak_pss = snap.pss_mb();
                                }
                                if snap.rss_mb() > peak_rss {
                                    peak_rss = snap.rss_mb();
                                }
                            }
                        }
                        if let Some(avail) = read_system_mem_available_kb() {
                            if avail < min_avail_kb {
                                min_avail_kb = avail;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }

                    (peak_pss, peak_rss, min_avail_kb)
                });

                let barrier = Arc::new(Barrier::new(n_branches));
                let mut tasks = Vec::with_capacity(n_branches);

                let trial_start = Instant::now();
                for i in 0..n_branches {
                    let branch = manifest.branches[i].clone();
                    let b_client = client.clone();
                    let b_url = engine_url.clone();
                    let b_model = manifest.model_id.clone();
                    let b_prefix = prefix_tokens.clone();
                    let b_barrier = barrier.clone();
                    let max_tok = args.max_tokens;

                    tasks.push(tokio::spawn(async move {
                        execute_single_branch(
                            b_client,
                            b_url,
                            b_model,
                            b_prefix,
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

                // Stop memory sampler and extract measured peaks
                stop_monitor.store(true, Ordering::Relaxed);
                let (peak_pss, peak_rss, min_avail_kb) = monitor_task.await.unwrap_or((0.0, 0.0, 0));

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

                let mut itl_vec: Vec<f64> = branch_results
                    .iter()
                    .filter(|b| b.success)
                    .map(|b| b.itl_p50_ms)
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
                    prefix_tokens: manifest.prefix_token_count,
                    ttft_p50_ms: calculate_percentile(&mut ttft_vec, 0.50),
                    ttft_p95_ms: calculate_percentile(&mut ttft_vec, 0.95),
                    ttft_p99_ms: calculate_percentile(&mut ttft_vec, 0.99),
                    itl_p50_ms: calculate_percentile(&mut itl_vec, 0.50),
                    itl_p95_ms: calculate_percentile(&mut itl_vec, 0.95),
                    aggregate_tokens_per_sec: agg_tok_per_sec,
                    total_wall_time_ms: wall_time_ms,
                    pss_mb_before,
                    pss_mb_peak: if peak_pss > 0.0 { Some(peak_pss) } else { None },
                    rss_mb_peak: if peak_rss > 0.0 { Some(peak_rss) } else { None },
                    min_mem_available_mb: if min_avail_kb < u64::MAX {
                        Some(min_avail_kb as f64 / 1024.0)
                    } else {
                        None
                    },
                };

                println!(
                    "  -> Completed: {}/{}, TTFT p50: {:.2}ms, ITL p50: {:.2}ms, Agg tok/s: {:.2}, Peak PSS: {:.2} MB",
                    completed,
                    n_branches,
                    summary.ttft_p50_ms,
                    summary.itl_p50_ms,
                    summary.aggregate_tokens_per_sec,
                    summary.pss_mb_peak.unwrap_or(0.0)
                );

                all_summaries.push(summary);
            }
        }
    }

    let summary_path = args.output_dir.join("summary.json");
    let mut file = File::create(&summary_path)?;
    writeln!(file, "{}", serde_json::to_string_pretty(&all_summaries)?)?;
    println!("Benchmark run complete. Summary saved to {}", summary_path.display());

    Ok(())
}
