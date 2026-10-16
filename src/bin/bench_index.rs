//! Writes (or with `--check`, compares) `docs/PERF_INDEX.md`, the generated discovery index.
//!
//! Usage: `bench_index [--root <repo>] [--check]`. With `--check` it writes nothing and exits 1
//! when the committed index differs from what the files in the repository produce.

use aien_benchmarks::perf_index::{generate, INDEX_PATH};
use clap::Parser;
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "bench_index")]
#[command(about = "Generate docs/PERF_INDEX.md from the files in this repository")]
struct Args {
    /// Repository root
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Compare instead of writing; exit 1 if the committed index is out of date
    #[arg(long)]
    check: bool,
}

fn main() {
    let args = Args::parse();
    let text = generate(&args.root);
    let path = args.root.join(INDEX_PATH);
    if args.check {
        let current = fs::read_to_string(&path).unwrap_or_default();
        if current != text {
            eprintln!(
                "bench_index: {INDEX_PATH} is out of date; run: cargo run --release --bin bench_index"
            );
            std::process::exit(1);
        }
        eprintln!("bench_index: {INDEX_PATH} is up to date");
        return;
    }
    if let Err(e) = fs::write(&path, text) {
        eprintln!("bench_index: cannot write {INDEX_PATH}: {e}");
        std::process::exit(2);
    }
    eprintln!("bench_index: wrote {INDEX_PATH}");
}
