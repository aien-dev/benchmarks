//! Runs `bench_verify_records` on the committed fixtures and bundles and checks its exact defect lines.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ROOT: &str = env!("CARGO_MANIFEST_DIR");
const BUNDLE: &str = "artifacts/gb10_canonical_1789907893_4d762";

fn verify(root: &Path, args: &[&str]) -> (i32, Vec<String>) {
    let out = Command::new(env!("CARGO_BIN_EXE_bench_verify_records"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("run bench_verify_records");
    let lines = String::from_utf8(out.stdout)
        .expect("utf-8 output")
        .lines()
        .map(str::to_string)
        .collect();
    (out.status.code().expect("exit code"), lines)
}

fn bad(name: &str, expected: &[&str]) {
    let path = format!("tests/fixtures/records/bad/{name}.json");
    let (code, lines) = verify(Path::new(ROOT), &[&path]);
    let want: Vec<String> = expected.iter().map(|l| format!("{path}: {l}")).collect();
    assert_eq!(lines, want, "defect lines for {name}");
    assert_eq!(code, 1, "{name} must fail");
}

#[test]
fn good_fixture_and_committed_evidence_pass() {
    let (code, lines) = verify(
        Path::new(ROOT),
        &["records", "artifacts", "tests/fixtures/records/good"],
    );
    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(code, 0);
}

#[test]
fn bad_commit() {
    bad(
        "bad_commit",
        &["source.commit: \"dfd5ea9\" is not a 40-character lowercase hex commit id"],
    );
}

#[test]
fn missing_field() {
    bad("missing_field", &["backend.model: missing"]);
}

#[test]
fn placeholder() {
    bad(
        "placeholder",
        &["backend.precision: placeholder value \"TBD\""],
    );
}

#[test]
fn status_outside_the_vocabulary() {
    bad(
        "bad_status",
        &["status: \"PASSED\" is not one of VERIFIED, EXPERIMENTAL, HISTORICAL, NOT_RUN, FAIL, SUPERSEDED"],
    );
}

#[test]
fn evidence_path_missing() {
    bad(
        "missing_evidence",
        &["evidence[0]: path does not exist: artifacts/gb10_canonical_1789907893_4d762/does_not_exist.json"],
    );
}

#[test]
fn verified_needs_clean_tree_and_full_provenance() {
    let nr = "not_recorded is only allowed in HISTORICAL or SUPERSEDED records";
    bad(
        "verified_dirty",
        &[
            &format!("backend.endpoint: {nr}"),
            &format!("build.profile: {nr}"),
            &format!("build.rustc: {nr}"),
            &format!("methodology.clock: {nr}"),
            &format!("methodology.samples: {nr}"),
            &format!("methodology.token_accounting: {nr}"),
            "source.dirty: VERIFIED requires dirty == false",
        ],
    );
}

#[test]
fn p95_from_one_sample() {
    bad(
        "p95_one_sample",
        &["metrics.cold_fork_to_first_token_ms_p95: p95 needs at least 20 samples, record has 1"],
    );
}

#[test]
fn stream_events_are_not_tokens() {
    bad(
        "events_as_tokens",
        &["metrics.decode_throughput_tok_per_sec: token metric with token_accounting events_not_tokens (stream events are not tokens)"],
    );
}

#[test]
fn non_numeric_values() {
    bad(
        "non_numeric",
        &[
            "metrics.physical_cow_pages_allocated: must be a finite number",
            "workload.prefix_tokens: must be a finite number",
        ],
    );
}

/// Copy the committed bundle to a scratch root and break it three ways.
#[test]
fn tampered_bundle_is_reported() {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("evidence_bundle_tamper");
    let _ = fs::remove_dir_all(&scratch);
    let dst = scratch.join(BUNDLE);
    fs::create_dir_all(&dst).unwrap();
    for e in fs::read_dir(Path::new(ROOT).join(BUNDLE)).unwrap() {
        let e = e.unwrap();
        fs::copy(e.path(), dst.join(e.file_name())).unwrap();
    }
    // Clean copy passes (control).
    assert_eq!(verify(&scratch, &["artifacts"]), (0, Vec::new()));

    let mut cold = fs::read(dst.join("branch_cold.json")).unwrap();
    cold.push(b'\n');
    fs::write(dst.join("branch_cold.json"), cold).unwrap();
    fs::remove_file(dst.join("branch_steady.json")).unwrap();
    fs::write(dst.join("extra.txt"), b"not listed\n").unwrap();
    let (code, lines) = verify(&scratch, &["artifacts"]);
    assert_eq!(
        lines,
        vec![
            format!("{BUNDLE}/branch_steady.json: SHA256SUMS: listed file is missing"),
            format!("{BUNDLE}/branch_cold.json: sha256: does not match SHA256SUMS"),
            format!("{BUNDLE}/extra.txt: SHA256SUMS: file is not listed"),
        ]
    );
    assert_eq!(code, 1);
    let _ = fs::remove_dir_all(&scratch);
}
