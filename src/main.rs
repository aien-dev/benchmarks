mod models;
mod svg_chart;

use clap::{Parser, Subcommand};
use models::BenchmarkData;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "aien-benchmarks")]
#[command(about = "Sovereign Systems Benchmark Suite CLI & SVG Generator")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, default_value = "data/benchmarks_20260919.json")]
    input: String,
}

#[derive(Subcommand)]
enum Commands {
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

fn print_report(data: &BenchmarkData) {
    println!("================================================================================");
    println!("  AIEN Sovereign Systems Performance Benchmark Report");
    println!("  Hardware: {} ({} {})", data.hardware.system, data.hardware.processor, data.hardware.architecture);
    println!("  Unified Memory: {} GB | OS: {}", data.hardware.memory_unified_gb, data.hardware.os);
    println!("  Timestamp: {}", data.timestamp);
    println!("================================================================================\n");

    println!("--- 1. Memory Resident Set Size (RSS) vs Python Agent Baseline ---");
    println!("{:<24} | {:<32} | {:>10} | {:>12}", "Service", "Architecture", "RSS (MB)", "Reduction");
    println!("{:-<24}-|-{:-<32}-|-{:-<10}-|-{:-<12}", "", "", "", "");
    for m in &data.memory_rss {
        let badge = if m.reduction_pct > 0.0 {
            format!("-{:.2}%", m.reduction_pct)
        } else {
            "Baseline".to_string()
        };
        println!("{:<24} | {:<32} | {:>10.2} | {:>12}", m.service, m.architecture, m.rss_mb, badge);
    }
    println!();

    println!("--- 2. HTTP Gateway Latency (p50 TTFB) & Concurrency Throughput ---");
    println!("{:<28} | {:<22} | {:>10} | {:>10} | {:>10}", "Endpoint", "Engine", "Req/sec", "p50 (ms)", "p95 (ms)");
    println!("{:-<28}-|-{:-<22}-|-{:-<10}-|-{:-<10}-|-{:-<10}", "", "", "", "", "");
    for l in &data.latency_concurrency {
        let label = format!("{} {}", l.service, l.endpoint);
        println!("{:<28} | {:<22} | {:>10.1} | {:>10.2} | {:>10.2}", label, l.engine, l.requests_per_sec, l.p50_ms, l.p95_ms);
    }
    println!();

    println!("--- 3. Local Neural Inference & SIMD Vectors (Grace Blackwell) ---");
    println!("{:<36} | {:<24} | {:>10} | {:>10}", "Workload", "Model / Kernel", "p50 (ms)", "p95 (ms)");
    println!("{:-<36}-|-{:-<24}-|-{:-<10}-|-{:-<10}", "", "", "", "");
    for n in &data.neural_inference {
        println!("{:<36} | {:<24} | {:>10.2} | {:>10.2}", n.workload, n.model, n.p50_ms, n.p95_ms);
    }
    println!("================================================================================");
}

fn main() {
    let cli = Cli::parse();
    let data = load_data(&cli.input);

    match cli.command.unwrap_or(Commands::Report) {
        Commands::Report => {
            print_report(&data);
        }
        Commands::Generate { output } => {
            let out_dir = Path::new(&output);
            fs::create_dir_all(out_dir).expect("Failed to create output directory");

            let mem_svg = svg_chart::generate_memory_chart(&data);
            let mem_path = out_dir.join("memory_comparison.svg");
            fs::write(&mem_path, mem_svg).expect("Failed to write memory SVG");
            println!("  [OK] Rendered {}", mem_path.display());

            let lat_svg = svg_chart::generate_latency_chart(&data);
            let lat_path = out_dir.join("latency_comparison.svg");
            fs::write(&lat_path, lat_svg).expect("Failed to write latency SVG");
            println!("  [OK] Rendered {}", lat_path.display());

            println!("All SVG benchmark charts successfully generated.");
        }
        Commands::Verify => {
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
        assert!(!mem_svg.contains('—'), "Must not contain em dash");
        assert!(!mem_svg.contains('–'), "Must not contain en dash");

        let lat_svg = svg_chart::generate_latency_chart(&data);
        assert!(lat_svg.contains("<svg"), "Must contain SVG opening tag");
        assert!(lat_svg.contains("</svg>"), "Must contain SVG closing tag");
        assert!(!lat_svg.contains('—'), "Must not contain em dash");
        assert!(!lat_svg.contains('–'), "Must not contain en dash");
    }
}
