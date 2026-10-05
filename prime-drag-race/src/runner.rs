//! `verify` (correctness sweep) and `run` (trial protocol + receipt).
use super::exec::{execute, Executed};
use super::receipt::*;
use super::reference;
use super::validate::{validate_run, Expect, Reject, Tags, Validated};
use super::{CONTRACT_VERSION, RECEIPT_SCHEMA};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Clone)]
pub struct ImplSpec {
    pub name: String,
    pub path: PathBuf,
    pub tags: Tags,
}

pub fn run_and_validate(
    spec: &ImplSpec,
    limit: u64,
    min_seconds: f64,
    audit: Option<u32>,
) -> (Option<Executed>, Result<Validated, Reject>) {
    let exp = Expect { name: spec.name.clone(), limit, min_seconds, audit, tags: spec.tags.clone() };
    match execute(&spec.path, limit, min_seconds, audit) {
        Ok(ex) => {
            let v = validate_run(&exp, &ex);
            (Some(ex), v)
        }
        Err(e) => (None, Err(Reject::Spawn(e.to_string()))),
    }
}

pub struct VerifyOutcome {
    pub checked: usize,
    pub failures: Vec<(u64, Reject)>,
}

pub fn verify(spec: &ImplSpec, limits: &[u64], audit: u32) -> VerifyOutcome {
    let mut failures = vec![];
    for &l in limits {
        let (_, r) = run_and_validate(spec, l, 0.0, Some(audit));
        if let Err(e) = r {
            eprintln!("FAIL limit {l}: {e}");
            failures.push((l, e));
        }
    }
    VerifyOutcome { checked: limits.len(), failures }
}

pub struct RunArgs {
    pub impls: Vec<ImplSpec>,
    pub trials: u32,
    pub warmup: u32,
    pub limit: u64,
    pub min_seconds: f64,
    pub audit_passes: u32,
    pub evidence_dir: PathBuf,
    pub sources: BTreeMap<String, String>,
    pub builds: BTreeMap<String, String>,
    pub runner_commit: Option<String>,
    pub quiet_file: PathBuf,
    pub argv: Vec<String>,
}

pub struct RunResult {
    pub pass: bool,
    pub receipt_path: PathBuf,
    pub receipt_sha: String,
    pub summary: String,
    pub upstream_lines: Vec<String>,
}

fn record(
    evidence: &std::path::Path,
    spec: &ImplSpec,
    kind: &str,
    index: u32,
    limit: u64,
    min_seconds: f64,
    audit: Option<u32>,
) -> std::io::Result<(Value, Result<Validated, Reject>)> {
    let before = loadavg();
    let (ex, res) = run_and_validate(spec, limit, min_seconds, audit);
    let after = loadavg();
    let mut rec = json!({
        "impl": spec.name, "kind": kind, "index": index, "limit": limit,
        "loadavg_before": before, "loadavg_after": after,
    });
    if let Some(ex) = &ex {
        rec["exit_code"] = json!(ex.exit_code);
        rec["wall_s"] = json!(ex.wall_s);
        rec["raw_line"] = json!(String::from_utf8_lossy(&ex.stdout).trim_end_matches('\n'));
        rec["stdout_sha256"] = json!(store_blob(evidence, &ex.stdout)?);
        rec["stderr_sha256"] = json!(store_blob(evidence, &ex.stderr)?);
        if let Some(r) = &ex.report {
            rec["report_sha256"] = json!(store_blob(evidence, r)?);
            rec["report"] = serde_json::from_slice(r).unwrap_or(Value::Null);
        }
        if let Some(b) = &ex.bitmap {
            rec["bitmap_sha256"] = json!(store_blob(evidence, b)?);
        }
    }
    match &res {
        Ok(v) => {
            rec["verdict"] = json!("PASS");
            rec["passes"] = json!(v.line.passes);
            rec["elapsed_s"] = json!(v.line.elapsed);
            if audit.is_none() {
                rec["passes_per_s"] = json!(v.line.passes as f64 / v.line.elapsed);
            }
        }
        Err(e) => {
            rec["verdict"] = json!("FAIL");
            rec["reject_kind"] = json!(e.kind());
            rec["reject_reason"] = json!(e.to_string());
        }
    }
    Ok((rec, res))
}

pub fn run(a: &RunArgs) -> Result<RunResult, String> {
    let evidence = check_evidence_dir(&a.evidence_dir)?;
    fs::create_dir_all(evidence.join("blobs")).map_err(|e| e.to_string())?;
    let io = |e: std::io::Error| e.to_string();

    let mut bin_sha = BTreeMap::new();
    for s in &a.impls {
        bin_sha.insert(s.name.clone(), sha256_file(&s.path).map_err(io)?);
    }
    let mut records: Vec<Value> = vec![];
    let mut samples: BTreeMap<String, Vec<f64>> = a.impls.iter().map(|s| (s.name.clone(), vec![])).collect();
    let mut failure: Option<String> = None;
    let mut upstream: Vec<String> = vec![];

    let mut step = |spec: &ImplSpec, kind: &str, idx: u32, audit: Option<u32>, records: &mut Vec<Value>| -> Result<Option<String>, String> {
        let (rec, res) = record(&evidence, spec, kind, idx, a.limit, a.min_seconds, audit).map_err(io)?;
        records.push(rec.clone());
        match res {
            Ok(v) => {
                if kind == "trial" {
                    samples.get_mut(&spec.name).unwrap().push(v.line.passes as f64 / v.line.elapsed);
                    upstream.push(rec["raw_line"].as_str().unwrap_or("").to_string());
                }
                Ok(None)
            }
            Err(e) => Ok(Some(format!("{} {kind} {idx}: {e}", spec.name))),
        }
    };

    'all: {
        for s in &a.impls {
            if let Some(f) = step(s, "audit", 0, Some(a.audit_passes), &mut records)? {
                failure = Some(f);
                break 'all;
            }
        }
        for w in 0..a.warmup {
            for s in &a.impls {
                if let Some(f) = step(s, "warmup", w, None, &mut records)? {
                    failure = Some(f);
                    break 'all;
                }
            }
        }
        for t in 0..a.trials {
            for s in &a.impls {
                if let Some(f) = step(s, "trial", t, None, &mut records)? {
                    failure = Some(f);
                    break 'all;
                }
            }
        }
    }

    let pass = failure.is_none();
    let mut impls = serde_json::Map::new();
    let mut summary_json = serde_json::Map::new();
    let mut human = String::new();
    for s in &a.impls {
        let rep = records.iter().find(|r| r["impl"] == json!(s.name) && r.get("report").is_some_and(|x| !x.is_null()));
        let env = rep.and_then(|r| r["report"].get("environment").cloned());
        let det = rep.and_then(|r| r["report"].get("build").cloned());
        impls.insert(
            s.name.clone(),
            json!({
                "name": s.name,
                "labels": {"line_label": format!("aien-{}", s.name), "algorithm": s.tags.algorithm,
                           "faithful": s.tags.faithful, "bits": s.tags.bits, "threads": s.tags.threads},
                "environment": env,
                "binary_sha256": bin_sha[&s.name],
                "source": a.sources.get(&s.name),
                "build_command": a.builds.get(&s.name),
                "build_reported_by_binary": det,
            }),
        );
        if pass {
            if let Some(st) = stats(&samples[&s.name]) {
                human.push_str(&format!(
                    "{:<16} median {:>12.2}  min {:>12.2}  max {:>12.2}  mean {:>12.2}  sd {:>10.2}  cv {:.4}  (n={})\n",
                    s.name, st.median, st.min, st.max, st.mean, st.stddev, st.cv, st.n
                ));
                summary_json.insert(s.name.clone(), st.to_json());
            }
        }
    }
    let runner_sha = std::env::current_exe().ok().and_then(|p| sha256_file(&p).ok());
    let receipt = json!({
        "schema": RECEIPT_SCHEMA,
        "contract_version": CONTRACT_VERSION,
        "verdict": if pass { "PASS" } else { "FAIL" },
        "reason": failure.clone().unwrap_or_else(|| "every audit and trial validated PASS".into()),
        "created_utc": chrono::Utc::now().to_rfc3339(),
        "implementation": Value::Object(impls),
        "runner": {"commit": runner_commit(a.runner_commit.as_deref()), "binary_sha256": runner_sha},
        "hardware": hardware(),
        "conditions": {
            "quiet_flag_file": a.quiet_file.display().to_string(),
            "quiet_flag_contents": fs::read_to_string(&a.quiet_file).ok().map(|s| s.trim().to_string()),
            "governors": governors(),
            "loadavg_at_start": records.first().map(|r| r["loadavg_before"].clone()),
            "loadavg_at_end": records.last().map(|r| r["loadavg_after"].clone()),
        },
        "correctness": {
            "reference_method": "deterministic Miller-Rabin on every odd n <= limit (bases 2,3,5,7 below 3215031751, else first 12 primes, u128 mulmod); full bitmap compare",
            "limit": a.limit,
            "audit_passes": a.audit_passes,
            "sweep": "run `prime_race verify` separately; not part of this receipt",
        },
        "protocol": {"trials": a.trials, "warmup": a.warmup, "min_seconds": a.min_seconds,
                     "order": "round-robin interleaved across implementations, sequential"},
        "trials": records,
        "summary": if pass { Value::Object(summary_json) } else { Value::Null },
        "upstream_lines": upstream,
        "reproduction": {"runner_argv": a.argv},
        "nonconforming": "NONCONFORMING for upstream PlummersSoftwareLLC/Primes drag-race (pinned a2899c96752b65ac1b08a0e1418ab7e708040ed0): AGPL-3.0-or-later license vs required BSD-3-Clause, language eligibility. Experimental.",
    });
    let (p, sha) = write_receipt(&evidence, &receipt).map_err(io)?;
    if !pass {
        human.push_str(&format!("FAIL: {}\n", failure.unwrap()));
    }
    Ok(RunResult { pass, receipt_path: p, receipt_sha: sha, summary: human, upstream_lines: upstream_clone(&receipt) })
}

fn upstream_clone(r: &Value) -> Vec<String> {
    r["upstream_lines"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// Sanity helper used by the CLI: expected pi for a limit from the reference.
pub fn reference_count(limit: u64) -> u64 {
    reference::prime_count(&reference::canonical_bytes(limit), limit)
}
