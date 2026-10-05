//! Negative and positive tests: fake implementations written as shell scripts.
use prime_race::exec::execute;
use prime_race::reference::canonical_bytes;
use prime_race::runner::{run, verify, ImplSpec, RunArgs};
use prime_race::validate::{validate_run, Expect, Reject, Tags};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LIMIT: u64 = 1000;
const MIN: f64 = 1.0;
static N: AtomicU64 = AtomicU64::new(0);

fn tags() -> Tags {
    Tags { algorithm: "base".into(), faithful: "yes".into(), bits: 1, threads: Some(1) }
}

#[derive(Clone)]
struct Fake {
    line: Option<String>,
    report: Option<String>,
    bitmap: Option<Vec<u8>>,
    code: i32,
}

fn line(label: &str, passes: u64, elapsed: f64) -> String {
    format!("{label};{passes};{elapsed:.6};1;algorithm=base,faithful=yes,bits=1\n")
}

fn report(label: &str, passes: u64, elapsed: f64, mode: &str) -> String {
    json!({
        "schema": "aien-prime-race/impl-report/v1", "impl": "fake", "label": label,
        "algorithm": "base", "faithful": "yes", "bits": 1, "threads": 1,
        "environment": "linux-host-cpu", "limit": LIMIT, "min_seconds": MIN, "mode": mode,
        "passes": passes, "elapsed_s": elapsed, "status": "COMPLETE", "error": null,
        "build": {}, "detail": {}
    })
    .to_string()
}

fn good() -> Fake {
    Fake {
        line: Some(line("aien-fake", 10, 1.5)),
        report: Some(report("aien-fake", 10, 1.5, "timed")),
        bitmap: Some(canonical_bytes(LIMIT)),
        code: 0,
    }
}

fn dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("pr-neg-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&d).unwrap();
    d
}

fn script(f: &Fake) -> PathBuf {
    let d = dir();
    let mut body = String::from(
        "#!/bin/sh\nB=; R=\nwhile [ $# -gt 0 ]; do case \"$1\" in --bitmap-out) B=\"$2\"; shift;; --report-out) R=\"$2\"; shift;; esac; shift; done\n",
    );
    if let Some(b) = &f.bitmap {
        fs::write(d.join("bm"), b).unwrap();
        body.push_str(&format!("cp '{}' \"$B\"\n", d.join("bm").display()));
    }
    if let Some(r) = &f.report {
        fs::write(d.join("rp"), r).unwrap();
        body.push_str(&format!("cp '{}' \"$R\"\n", d.join("rp").display()));
    }
    if let Some(l) = &f.line {
        fs::write(d.join("ln"), l).unwrap();
        body.push_str(&format!("cat '{}'\n", d.join("ln").display()));
    }
    body.push_str(&format!("exit {}\n", f.code));
    let p = d.join("impl.sh");
    fs::write(&p, body).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn check(f: &Fake, audit: Option<u32>) -> Result<(), Reject> {
    let p = script(f);
    let ex = execute(&p, LIMIT, MIN, audit).unwrap();
    let exp = Expect { name: "fake".into(), limit: LIMIT, min_seconds: MIN, audit, tags: tags() };
    validate_run(&exp, &ex).map(|_| ())
}

fn with_bitmap(mut f: Fake, edit: impl FnOnce(&mut Vec<u8>)) -> Fake {
    edit(f.bitmap.as_mut().unwrap());
    f
}

fn kind(r: Result<(), Reject>) -> String {
    r.expect_err("must be rejected").kind().to_string()
}

fn bit(b: &mut [u8], k: usize) {
    b[k / 8] ^= 1 << (k % 8);
}

#[test]
fn correct_fake_passes() {
    check(&good(), None).unwrap();
}

#[test]
fn wrong_bit() {
    let f = with_bitmap(good(), |b| bit(b, 10)); // 21
    match check(&f, None).unwrap_err() {
        Reject::BitmapDiff(d) => assert_eq!((d.odd, d.tail, d.expected_prime), (21, false, false)),
        e => panic!("{e}"),
    }
}

#[test]
fn missing_prime() {
    let f = with_bitmap(good(), |b| bit(b, 498)); // 997 cleared
    match check(&f, None).unwrap_err() {
        Reject::BitmapDiff(d) => assert_eq!((d.odd, d.expected_prime), (997, true)),
        e => panic!("{e}"),
    }
}

#[test]
fn extra_composite() {
    let f = with_bitmap(good(), |b| bit(b, 499)); // 999 set
    match check(&f, None).unwrap_err() {
        Reject::BitmapDiff(d) => assert_eq!((d.odd, d.expected_prime), (999, false)),
        e => panic!("{e}"),
    }
}

#[test]
fn tail_garbage() {
    let f = with_bitmap(good(), |b| bit(b, 505));
    match check(&f, None).unwrap_err() {
        Reject::BitmapDiff(d) => assert!(d.tail),
        e => panic!("{e}"),
    }
}

#[test]
fn truncated_bitmap() {
    let f = with_bitmap(good(), |b| {
        b.truncate(b.len() - 8);
    });
    assert_eq!(kind(check(&f, None)), "bitmap_length");
}

#[test]
fn all_zero_and_all_ones() {
    let z = with_bitmap(good(), |b| b.iter_mut().for_each(|x| *x = 0));
    assert_eq!(kind(check(&z, None)), "bitmap_diff");
    let o = with_bitmap(good(), |b| b.iter_mut().for_each(|x| *x = 0xff));
    assert_eq!(kind(check(&o, None)), "bitmap_diff");
}

fn audit_fake(stale: bool) -> Fake {
    let one = canonical_bytes(LIMIT);
    let mut all = vec![];
    for i in 0..3 {
        let mut b = one.clone();
        if stale && i == 1 {
            bit(&mut b, 10);
        }
        all.extend(b);
    }
    Fake {
        line: Some(line("aien-fake", 3, 0.2)),
        report: Some(report("aien-fake", 3, 0.2, "audit")),
        bitmap: Some(all),
        code: 0,
    }
}

#[test]
fn audit_good_and_stale_pass() {
    check(&audit_fake(false), Some(3)).unwrap();
    match check(&audit_fake(true), Some(3)).unwrap_err() {
        Reject::AuditPassDiff { pass, .. } => assert_eq!(pass, 1),
        e => panic!("{e}"),
    }
}

#[test]
fn exit_code_2() {
    let mut f = good();
    f.code = 2;
    assert_eq!(kind(check(&f, None)), "exit_code");
}

#[test]
fn no_stdout_line() {
    let mut f = good();
    f.line = None;
    assert_eq!(kind(check(&f, None)), "no_stdout_line");
}

#[test]
fn two_lines() {
    let mut f = good();
    let l = f.line.clone().unwrap();
    f.line = Some(format!("{l}{l}"));
    assert_eq!(kind(check(&f, None)), "multiple_stdout_lines");
}

#[test]
fn malformed_line() {
    let mut f = good();
    f.line = Some("aien-fake;10;1.5;1;algorithm=base,faithful=yes,bits=1\n".into());
    assert_eq!(kind(check(&f, None)), "malformed_line");
}

#[test]
fn passes_zero() {
    let mut f = good();
    f.line = Some(line("aien-fake", 0, 1.5));
    f.report = Some(report("aien-fake", 0, 1.5, "timed"));
    assert_eq!(kind(check(&f, None)), "passes_zero");
}

#[test]
fn elapsed_below_min() {
    let mut f = good();
    f.line = Some(line("aien-fake", 10, 0.5));
    f.report = Some(report("aien-fake", 10, 0.5, "timed"));
    assert_eq!(kind(check(&f, None)), "elapsed_below_min");
}

#[test]
fn line_report_mismatch() {
    let mut f = good();
    f.report = Some(report("aien-fake", 11, 1.5, "timed"));
    assert_eq!(kind(check(&f, None)), "line_report_mismatch");
}

#[test]
fn label_mismatch() {
    let mut f = good();
    f.line = Some(line("aien-other", 10, 1.5));
    f.report = Some(report("aien-other", 10, 1.5, "timed"));
    assert_eq!(kind(check(&f, None)), "label_mismatch");
    // Tags that differ from the declaration are also label mismatches.
    let mut g = good();
    g.line = Some("aien-fake;10;1.500000;1;algorithm=other,faithful=yes,bits=1\n".into());
    assert_eq!(kind(check(&g, None)), "label_mismatch");
}

#[test]
fn failed_status_rejected() {
    let mut f = good();
    f.report = Some(report("aien-fake", 10, 1.5, "timed").replace("COMPLETE", "EXEC_FAILED"));
    assert_eq!(kind(check(&f, None)), "status_not_complete");
}

#[test]
fn verify_subcommand_logic_and_run_receipt() {
    // verify: the fake always emits the LIMIT=1000 bitmap, so only that limit can pass.
    let mut f = audit_fake(false);
    f.report = Some(report("aien-fake", 3, 0.2, "audit"));
    let p = script(&f);
    let spec = ImplSpec { name: "fake".into(), path: p, tags: tags() };
    let out = verify(&spec, &[LIMIT], 3);
    assert!(out.failures.is_empty());
    let out = verify(&spec, &[LIMIT + 2], 3);
    assert_eq!(out.failures.len(), 1);

    // run: audit uses the same limit, so give a script that handles both modes via two bitmaps is
    // not possible with fixtures; use trials=0 warmup=0 to exercise the audit + receipt path.
    let ev = std::env::temp_dir().join(format!("pr-ev-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let args = RunArgs {
        impls: vec![spec],
        trials: 0,
        warmup: 0,
        limit: LIMIT,
        min_seconds: MIN,
        audit_passes: 3,
        evidence_dir: ev.clone(),
        sources: BTreeMap::new(),
        builds: BTreeMap::new(),
        runner_commit: Some("test".into()),
        quiet_file: "/nonexistent".into(),
        argv: vec!["prime_race".into()],
    };
    let r = run(&args).unwrap();
    assert!(r.pass, "{}", r.summary);
    let body = fs::read_to_string(&r.receipt_path).unwrap();
    assert!(body.contains("aien-prime-race/receipt/v1"));
    let _ = fs::remove_dir_all(&ev);
}

#[test]
fn run_failure_keeps_evidence_and_no_stats() {
    let f = audit_fake(true);
    let spec = ImplSpec { name: "fake".into(), path: script(&f), tags: tags() };
    let ev = std::env::temp_dir().join(format!("pr-ev-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let args = RunArgs {
        impls: vec![spec],
        trials: 2,
        warmup: 0,
        limit: LIMIT,
        min_seconds: MIN,
        audit_passes: 3,
        evidence_dir: ev.clone(),
        sources: BTreeMap::new(),
        builds: BTreeMap::new(),
        runner_commit: Some("test".into()),
        quiet_file: "/nonexistent".into(),
        argv: vec![],
    };
    let r = run(&args).unwrap();
    assert!(!r.pass);
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&r.receipt_path).unwrap()).unwrap();
    assert_eq!(v["verdict"], "FAIL");
    assert!(v["summary"].is_null());
    assert!(fs::read_dir(ev.join("blobs")).unwrap().count() > 0);
    let _ = fs::remove_dir_all(&ev);
}
