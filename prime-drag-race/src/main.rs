use prime_race::runner::{reference_count, run, verify, ImplSpec, RunArgs};
use prime_race::sweep::{default_sweep, parse_limits};
use prime_race::validate::Tags;
use clap::{Parser, Subcommand};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "prime_race", about = "AIEN Prime Drag Race suite runner (contract version 1)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Correctness sweep of one implementation.
    Verify {
        #[arg(long = "impl")]
        imp: PathBuf,
        #[arg(long)]
        name: String,
        /// Declared tags, e.g. algorithm=base,faithful=yes,bits=1,threads=1
        #[arg(long)]
        declare: String,
        /// Comma list with a..b ranges. Default: the contract sweep.
        #[arg(long)]
        limits: Option<String>,
        #[arg(long, default_value_t = 3)]
        audit_passes: u32,
    },
    /// Trial protocol with receipt.
    Run {
        /// NAME=PATH, repeatable
        #[arg(long = "impl", required = true)]
        imps: Vec<String>,
        /// NAME=algorithm=..,faithful=..,bits=..[,threads=..], one per impl
        #[arg(long = "declare", required = true)]
        declares: Vec<String>,
        /// NAME=REPO@COMMIT, repeatable
        #[arg(long = "source")]
        sources: Vec<String>,
        /// NAME=build command and flags, repeatable
        #[arg(long = "build")]
        builds: Vec<String>,
        #[arg(long, default_value_t = 10)]
        trials: u32,
        #[arg(long, default_value_t = 1)]
        warmup: u32,
        #[arg(long, default_value_t = 1_000_000)]
        limit: u64,
        #[arg(long, default_value_t = 5.0)]
        min_seconds: f64,
        #[arg(long, default_value_t = 5)]
        audit_passes: u32,
        #[arg(long)]
        evidence_dir: PathBuf,
        #[arg(long)]
        runner_commit: Option<String>,
        #[arg(long)]
        quiet_flag_file: Option<PathBuf>,
    },
}

fn kv(s: &str) -> Result<(String, String), String> {
    s.split_once('=').map(|(a, b)| (a.to_string(), b.to_string())).ok_or_else(|| format!("expected NAME=VALUE, got {s}"))
}

fn main() {
    let code = match real_main() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            64
        }
    };
    std::process::exit(code);
}

fn real_main() -> Result<i32, String> {
    let argv: Vec<String> = std::env::args().collect();
    match Cli::parse().cmd {
        Cmd::Verify { imp, name, declare, limits, audit_passes } => {
            let limits = match limits {
                Some(t) => parse_limits(&t)?,
                None => default_sweep(),
            };
            let spec = ImplSpec { name, path: imp, tags: Tags::parse(&declare)? };
            let out = verify(&spec, &limits, audit_passes);
            println!(
                "verify {}: {} limits, audit passes {}, {} failures{}",
                spec.name,
                out.checked,
                audit_passes,
                out.failures.len(),
                if out.failures.is_empty() { " -> PASS" } else { " -> FAIL" }
            );
            if let Some(l) = limits.iter().find(|&&l| l == 1_000_000) {
                println!("reference pi({l}) = {}", reference_count(*l));
            }
            Ok(if out.failures.is_empty() { 0 } else { 1 })
        }
        Cmd::Run { imps, declares, sources, builds, trials, warmup, limit, min_seconds, audit_passes, evidence_dir, runner_commit, quiet_flag_file } => {
            let decl: BTreeMap<String, String> = declares.iter().map(|s| kv(s)).collect::<Result<_, _>>()?;
            let mut specs = vec![];
            for s in &imps {
                let (name, path) = kv(s)?;
                let tags = Tags::parse(decl.get(&name).ok_or(format!("no --declare for {name}"))?)?;
                specs.push(ImplSpec { name, path: path.into(), tags });
            }
            let home = std::env::var("HOME").unwrap_or_default();
            let args = RunArgs {
                impls: specs,
                trials,
                warmup,
                limit,
                min_seconds,
                audit_passes,
                evidence_dir,
                sources: sources.iter().map(|s| kv(s)).collect::<Result<_, _>>()?,
                builds: builds.iter().map(|s| kv(s)).collect::<Result<_, _>>()?,
                runner_commit,
                quiet_file: quiet_flag_file.unwrap_or_else(|| PathBuf::from(home).join("workspace/.spark-quiet")),
                argv,
            };
            let r = run(&args)?;
            for l in &r.upstream_lines {
                println!("{l}");
            }
            println!("\n{}", r.summary);
            println!("verdict: {}", if r.pass { "PASS" } else { "FAIL" });
            println!("receipt: {} (sha256 {})", r.receipt_path.display(), r.receipt_sha);
            Ok(if r.pass { 0 } else { 1 })
        }
    }
}
