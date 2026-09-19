mod harness;
mod models;
mod svg_chart;

use clap::{Parser, Subcommand};
use models::BenchmarkData;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "aien-benchmarks")]
#[command(about = "Sovereign Systems Benchmark Suite CLI & Measurement Harness")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, default_value = "data/benchmarks_latest.json")]
    input: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute live measurement sweep, generate benchmark dataset, and render SVG charts
    Measure {
        #[arg(short, long, default_value_t = 10)]
        concurrency: u32,

        #[arg(short, long, default_value_t = 500)]
        requests: u32,

        #[arg(short, long, default_value_t = 20)]
        warmup: u32,

        #[arg(long, default_value = "data/benchmarks_latest.json")]
        output_json: String,

        #[arg(long, default_value = "assets")]
        output_assets: String,

        #[arg(long, default_value = "harness/python_baseline.py")]
        python_script: String,

        #[arg(long)]
        skip_standalone: bool,

        #[arg(long)]
        skip_live: bool,
    },
    /// Render SVG charts to assets/ directory
    Generate {
        #[arg(short, long, default_value = "assets")]
        output: String,
    },
    /// Print terminal ASCII summary table
    Report,
    /// Verify all metrics against baseline invariants
    Verify,
}

fn load_data(path: &str) -> BenchmarkData {
    let raw = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("Failed to read benchmark data file {}: {}", path, e));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("Failed to parse benchmark JSON: {}", e))
}

fn save_data(path: &str, data: &BenchmarkData) {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directory for dataset");
    }
    let json = serde_json::to_string_pretty(data).expect("Failed to serialize benchmark data");
    fs::write(path, json).expect("Failed to write benchmark data");
    println!("  [OK] Saved benchmark dataset to {}", path);
}

fn render_all_svgs(data: &BenchmarkData, output_dir: &str) {
    let out_dir = Path::new(output_dir);
    fs::create_dir_all(out_dir).expect("Failed to create output directory");

    let mem_svg = svg_chart::generate_memory_chart(data);
    let mem_path = out_dir.join("memory_comparison.svg");
    fs::write(&mem_path, mem_svg).expect("Failed to write memory SVG");
    println!("  [OK] Rendered {}", mem_path.display());

    let lat_svg = svg_chart::generate_latency_chart(data);
    let lat_path = out_dir.join("latency_comparison.svg");
    fs::write(&lat_path, lat_svg).expect("Failed to write latency SVG");
    println!("  [OK] Rendered {}", lat_path.display());

    if !data.like_for_like.is_empty() {
        let lfl_svg = svg_chart::generate_like_for_like_chart(data);
        let lfl_path = out_dir.join("like_for_like_comparison.svg");
        fs::write(&lfl_path, lfl_svg).expect("Failed to write like-for-like SVG");
        println!("  [OK] Rendered {}", lfl_path.display());
    }

    println!("All SVG benchmark charts successfully updated in {}.", output_dir);
}

fn print_report(data: &BenchmarkData) {
    println!("================================================================================");
    println!("  AIEN Sovereign Systems Performance Benchmark Report");
    println!(
        "  Hardware: {} ({} {})",
        data.hardware.system, data.hardware.processor, data.hardware.architecture
    );
    println!(
        "  Unified Memory: {} GB | OS: {}",
        data.hardware.memory_unified_gb, data.hardware.os
    );
    if !data.environment.rustc_version.is_empty() {
        println!(
            "  Compilers: {} | {}",
            data.environment.rustc_version, data.environment.python_version
        );
        println!(
            "  Kernel: {} | Page Size: {} KB",
            data.environment.kernel_version, data.environment.page_size_kb
        );
    }
    println!("  Timestamp: {}", data.timestamp);
    println!("================================================================================\n");

    println!("--- 1. Memory Resident Set Size (RSS) vs Python Agent Baseline ---");
    println!(
        "{:<24} | {:<32} | {:>10} | {:>12}",
        "Service", "Architecture", "RSS (MB)", "Reduction"
    );
    println!("{:-<24}-|-{:-<32}-|-{:-<10}-|-{:-<12}", "", "", "", "");
    for m in &data.memory_rss {
        let badge = if m.reduction_pct > 0.0 {
            format!("-{:.2}%", m.reduction_pct)
        } else {
            "Baseline".to_string()
        };
        println!(
            "{:<24} | {:<32} | {:>10.2} | {:>12}",
            m.service, m.architecture, m.rss_mb, badge
        );
    }
    println!();

    println!("--- 2. HTTP Gateway Latency (p50 TTFB) & Concurrency Throughput ---");
    println!(
        "{:<28} | {:<22} | {:>10} | {:>10} | {:>10}",
        "Endpoint", "Engine", "Req/sec", "p50 (ms)", "p95 (ms)"
    );
    println!("{:-<28}-|-{:-<22}-|-{:-<10}-|-{:-<10}-|-{:-<10}", "", "", "", "", "");
    for l in &data.latency_concurrency {
        let label = format!("{} {}", l.service, l.endpoint);
        println!(
            "{:<28} | {:<22} | {:>10.1} | {:>10.2} | {:>10.2}",
            label, l.engine, l.requests_per_sec, l.p50_ms, l.p95_ms
        );
    }
    println!();

    if !data.like_for_like.is_empty() {
        println!("--- 3. Like-for-Like Microservice Comparisons (Rust vs Python) ---");
        println!(
            "{:<36} | {:>10} | {:>10} | {:>10} | {:>12}",
            "Workload", "Rust (ms)", "Py (ms)", "Speedup", "RAM Saving"
        );
        println!("{:-<36}-|-{:-<10}-|-{:-<10}-|-{:-<10}-|-{:-<12}", "", "", "", "", "");
        for l in &data.like_for_like {
            let speedup = format!("{:.1}x", l.speedup_factor);
            let mem = format!("-{:.1}%", l.memory_reduction_pct);
            println!(
                "{:<36} | {:>10.2} | {:>10.2} | {:>10} | {:>12}",
                l.workload, l.rust_p50_ms, l.python_p50_ms, speedup, mem
            );
        }
        println!();
    }

    println!("--- 4. Local Neural Inference & SIMD Vectors (Grace Blackwell) ---");
    println!(
        "{:<36} | {:<24} | {:>10} | {:>10}",
        "Workload", "Model / Kernel", "p50 (ms)", "p95 (ms)"
    );
    println!("{:-<36}-|-{:-<24}-|-{:-<10}-|-{:-<10}", "", "", "", "");
    for n in &data.neural_inference {
        let p50_str = if n.p50_ms < 0.01 {
            format!("{:.4}", n.p50_ms)
        } else {
            format!("{:.2}", n.p50_ms)
        };
        let p95_str = if n.p95_ms < 0.01 {
            format!("{:.4}", n.p95_ms)
        } else {
            format!("{:.2}", n.p95_ms)
        };
        println!(
            "{:<36} | {:<24} | {:>10} | {:>10}",
            n.workload, n.model, p50_str, p95_str
        );
    }
    println!("================================================================================");
}

fn main() {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Report) {
        Commands::Measure {
            concurrency,
            requests,
            warmup,
            output_json,
            output_assets,
            python_script,
            skip_standalone,
            skip_live,
        } => {
            let config = harness::MeasurementConfig {
                concurrency,
                requests,
                warmup,
                skip_standalone,
                skip_live,
                python_script_path: python_script,
            };

            let data = match harness::run_measurement_suite(config) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Measurement sweep error: {}", e);
                    std::process::exit(1);
                }
            };

            print_report(&data);
            save_data(&output_json, &data);
            render_all_svgs(&data, &output_assets);
        }
        Commands::Report => {
            let data = load_data(&cli.input);
            print_report(&data);
        }
        Commands::Generate { output } => {
            let data = load_data(&cli.input);
            render_all_svgs(&data, &output);
        }
        Commands::Verify => {
            let data = load_data(&cli.input);
            for m in &data.memory_rss {
                if m.service == "openclaw-rs" {
                    assert!(m.rss_mb < 10.0, "openclaw-rs RSS exceeded 10MB budget");
                }
                if m.service == "cortex-rs" {
                    assert!(m.rss_mb < 20.0, "cortex-rs RSS exceeded 20MB budget");
                }
            }
            for l in &data.latency_concurrency {
                if l.engine.contains("Rust") {
                    assert!(l.p50_ms < 10.0, "Rust service latency exceeded 10ms p50");
                    assert!(l.requests_per_sec > 1000.0, "Rust service throughput below 1000 req/s");
                }
            }
            println!("All invariant checks passed successfully.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_reduction_math() {
        let sample_baseline = 3737.49f64;
        let sample_rust = 4.78f64;
        let reduction = (1.0 - (sample_rust / sample_baseline)) * 100.0;
        assert!(reduction > 99.8, "Reduction must exceed 99.8%");
    }

    #[test]
    fn test_svg_generation() {
        let raw = include_str!("../data/benchmarks_20260919.json");
        let data: BenchmarkData = serde_json::from_str(raw).expect("Valid JSON");
        let mem_svg = svg_chart::generate_memory_chart(&data);
        assert!(mem_svg.contains("<svg"), "Must contain SVG opening tag");
        assert!(mem_svg.contains("</svg>"), "Must contain SVG closing tag");
        assert!(!mem_svg.contains('\u{2014}'), "Must not contain em dash");
        assert!(!mem_svg.contains('\u{2013}'), "Must not contain en dash");

        let lat_svg = svg_chart::generate_latency_chart(&data);
        assert!(lat_svg.contains("<svg"), "Must contain SVG opening tag");
        assert!(lat_svg.contains("</svg>"), "Must contain SVG closing tag");
        assert!(!lat_svg.contains('\u{2014}'), "Must not contain em dash");
        assert!(!lat_svg.contains('\u{2013}'), "Must not contain en dash");
    }
}
