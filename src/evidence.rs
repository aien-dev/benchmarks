//! Offline evidence-integrity checks for benchmark records (`aien-bench-record/1`) and SHA256SUMS artifact bundles.
//!
//! Nothing here touches the network, a GPU or a live server: it reads files under the
//! repository root and reports one [`Defect`] per problem (`path: field: reason`).
//!
//! Rules for a record (a JSON file whose `schema` is [`RECORD_SCHEMA`]):
//! - every contract field is present with the right type; `status`, `build.profile` and
//!   `methodology.token_accounting` use their fixed vocabularies;
//! - `source.commit` is a 40-character lowercase hex commit id;
//! - no string anywhere is a placeholder ([`PLACEHOLDERS`], compared trimmed and case-blind);
//! - `metrics` is non-empty and every value is a finite number; every metric key has an entry
//!   in `methodology.definitions`; `methodology.samples >= 1`;
//! - a percentile metric (a key segment `pNN`) needs enough samples for that percentile to
//!   differ from the maximum: `ceil(100 / (100 - NN))` (p50 needs 2, p95 needs 20, p99 needs 100);
//! - with `token_accounting == "events_not_tokens"` no metric key may claim tokens;
//! - every `evidence` entry is a relative path that exists in the repository; only `NOT_RUN`
//!   records may have empty evidence, and they carry no evidence, no metrics and `samples == 0`;
//! - `VERIFIED` requires `source.dirty == false`;
//! - [`NOT_RECORDED`] is an honest "the original run did not record this" for a fixed set of
//!   provenance fields, allowed only in `HISTORICAL` and `SUPERSEDED` records and only when a
//!   `limitations` entry names the field.
//!
//! A directory holding a `SHA256SUMS` file is an immutable artifact bundle: every listed file
//! must exist and match its digest, every file in the directory must be listed, and its JSON
//! files that are not records get only the checks they share with records (placeholders, and
//! `*_git_sha` fields must be full commit ids). Bundles are reported, never rewritten.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The record contract this module validates.
pub const RECORD_SCHEMA: &str = "aien-bench-record/1";

/// Status vocabulary, in the order the README explains it.
pub const STATUSES: [&str; 6] = [
    "VERIFIED",
    "EXPERIMENTAL",
    "HISTORICAL",
    "NOT_RUN",
    "FAIL",
    "SUPERSEDED",
];

/// String values that stand in for a missing fact (compared trimmed, lowercase).
pub const PLACEHOLDERS: [&str; 7] = ["tbd", "todo", "unknown", "n/a", "", "example", "xxx"];

/// The one sanctioned value for "the original run did not record this".
pub const NOT_RECORDED: &str = "not_recorded";

/// Fields that may hold [`NOT_RECORDED`] (HISTORICAL and SUPERSEDED records only).
pub const NOT_RECORDED_FIELDS: [&str; 13] = [
    "source.dirty",
    "build.profile",
    "build.rustc",
    "hardware.host",
    "hardware.cpu",
    "hardware.gpu",
    "backend.engine",
    "backend.model",
    "backend.precision",
    "backend.endpoint",
    "methodology.clock",
    "methodology.token_accounting",
    "methodology.samples",
];

const REQUIRED_STRINGS: [&str; 12] = [
    "source.repo",
    "source.commit",
    "build.rustc",
    "workload.name",
    "hardware.host",
    "hardware.cpu",
    "hardware.gpu",
    "backend.engine",
    "backend.model",
    "backend.precision",
    "backend.endpoint",
    "methodology.clock",
];

const PROFILES: [&str; 2] = ["release", "debug"];
const TOKEN_ACCOUNTING: [&str; 2] = ["backend_usage", "events_not_tokens"];

/// One problem found in one file.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Defect {
    pub path: String,
    pub field: String,
    pub reason: String,
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.path, self.field, self.reason)
    }
}

/// Result of a run over one or more paths.
#[derive(Debug, Default)]
pub struct Report {
    pub defects: Vec<Defect>,
    pub records: usize,
    pub bundles: usize,
    pub bundle_files: usize,
    /// Informational lines (for example a default directory that does not exist).
    pub notes: Vec<String>,
}

/// Validate every target (a file or a directory, relative to `root`).
/// A target that does not exist is a note, not a defect, so the default `records/` may be absent.
pub fn verify_paths(root: &Path, targets: &[String]) -> Report {
    let mut report = Report::default();
    for target in targets {
        let clean = target.trim_end_matches('/');
        let full = root.join(clean);
        if !full.exists() {
            report
                .notes
                .push(format!("{clean}: skipped: not present under the root"));
            continue;
        }
        let mut files = Vec::new();
        if full.is_dir() {
            walk(&full, &mut files);
        } else {
            files.push(full.clone());
        }
        files.sort();
        // Bundle directories first: their files are checked by digest.
        let mut bundle_dirs: Vec<PathBuf> = files
            .iter()
            .filter(|f| f.file_name().is_some_and(|n| n == "SHA256SUMS"))
            .filter_map(|f| f.parent().map(Path::to_path_buf))
            .collect();
        bundle_dirs.sort();
        bundle_dirs.dedup();
        for dir in &bundle_dirs {
            report.bundles += 1;
            report.bundle_files += check_bundle(root, dir, &mut report.defects);
        }
        for file in &files {
            if file.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let display = display_path(root, file);
            let in_bundle = file
                .parent()
                .is_some_and(|p| bundle_dirs.iter().any(|d| d == p));
            let text = match fs::read_to_string(file) {
                Ok(t) => t,
                Err(e) => {
                    push(
                        &mut report.defects,
                        &display,
                        "(file)",
                        format!("cannot read: {e}"),
                    );
                    continue;
                }
            };
            let value: Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(e) => {
                    push(
                        &mut report.defects,
                        &display,
                        "(file)",
                        format!("invalid JSON: {e}"),
                    );
                    continue;
                }
            };
            let is_record = value.get("schema").and_then(Value::as_str) == Some(RECORD_SCHEMA);
            let mut found = Vec::new();
            if is_record {
                report.records += 1;
                check_record(root, &display, &value, &mut found);
            } else if in_bundle {
                check_legacy_bundle_json(&display, &value, &mut found);
            } else {
                match value.get("schema") {
                    None => push(
                        &mut found,
                        &display,
                        "schema",
                        format!(
                            "missing (not an {RECORD_SCHEMA} record and not inside a SHA256SUMS bundle)"
                        ),
                    ),
                    Some(s) => push(
                        &mut found,
                        &display,
                        "schema",
                        format!("{} is not {RECORD_SCHEMA}", s),
                    ),
                }
            }
            found.sort();
            found.dedup();
            report.defects.extend(found);
        }
    }
    report
}

/// Validate one parsed record. `path` is only used for the defect lines.
pub fn check_record(root: &Path, path: &str, v: &Value, out: &mut Vec<Defect>) {
    let mut d = |field: &str, reason: String| push(out, path, field, reason);
    if !v.is_object() {
        d("(file)", "top level is not a JSON object".into());
        return;
    }
    match v.get("schema").and_then(Value::as_str) {
        None => d("schema", "missing".into()),
        Some(s) if s != RECORD_SCHEMA => d("schema", format!("\"{s}\" is not {RECORD_SCHEMA}")),
        Some(_) => {}
    }
    let status = match v.get("status") {
        None => {
            d("status", "missing".into());
            None
        }
        Some(Value::String(s)) if STATUSES.contains(&s.as_str()) => Some(s.as_str()),
        Some(Value::String(s)) => {
            d(
                "status",
                format!("\"{s}\" is not one of {}", STATUSES.join(", ")),
            );
            None
        }
        Some(_) => {
            d("status", "must be a string".into());
            None
        }
    };
    let lenient = matches!(status, Some("HISTORICAL") | Some("SUPERSEDED"));

    let limitations: Vec<&str> = match v.get("limitations") {
        None => {
            d("limitations", "missing".into());
            Vec::new()
        }
        Some(Value::Array(a)) => {
            for (i, x) in a.iter().enumerate() {
                if !x.is_string() {
                    d(&format!("limitations[{i}]"), "must be a string".into());
                }
            }
            a.iter().filter_map(Value::as_str).collect()
        }
        Some(_) => {
            d("limitations", "must be an array of strings".into());
            Vec::new()
        }
    };

    for f in REQUIRED_STRINGS {
        match get(v, f) {
            None => d(f, "missing".into()),
            Some(x) if !x.is_string() => d(f, "must be a string".into()),
            Some(_) => {}
        }
    }
    if let Some(c) = get(v, "source.commit").and_then(Value::as_str) {
        if !is_commit(c) && !is_placeholder(c) {
            d(
                "source.commit",
                format!("\"{c}\" is not a 40-character lowercase hex commit id"),
            );
        }
    }
    let dirty = match get(v, "source.dirty") {
        None => {
            d("source.dirty", "missing".into());
            None
        }
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::String(s)) if s == NOT_RECORDED => None,
        Some(_) => {
            d("source.dirty", "must be true or false".into());
            None
        }
    };
    check_enum(v, "build.profile", &PROFILES, &mut d);
    check_enum(v, "methodology.token_accounting", &TOKEN_ACCOUNTING, &mut d);

    let samples = match get(v, "methodology.samples") {
        None => {
            d("methodology.samples", "missing".into());
            Err(())
        }
        Some(Value::String(s)) if s == NOT_RECORDED => Ok(None),
        Some(x) => match x.as_u64() {
            Some(n) => Ok(Some(n)),
            None => {
                d(
                    "methodology.samples",
                    "must be a non-negative integer".into(),
                );
                Err(())
            }
        },
    };

    match get(v, "workload") {
        Some(Value::Object(w)) => {
            for (k, x) in w {
                if k != "name" && !is_finite_number(x) {
                    d(&format!("workload.{k}"), "must be a finite number".into());
                }
            }
        }
        Some(_) => d("workload", "must be an object".into()),
        None => {}
    }

    let definitions = match get(v, "methodology.definitions") {
        None => {
            d("methodology.definitions", "missing".into());
            None
        }
        Some(Value::Object(m)) => {
            for (k, x) in m {
                if !x.is_string() {
                    d(
                        &format!("methodology.definitions.{k}"),
                        "must be a string".into(),
                    );
                }
            }
            Some(m)
        }
        Some(_) => {
            d("methodology.definitions", "must be an object".into());
            None
        }
    };

    let metrics = match v.get("metrics") {
        None => {
            d("metrics", "missing".into());
            None
        }
        Some(Value::Object(m)) => Some(m),
        Some(_) => {
            d("metrics", "must be an object".into());
            None
        }
    };
    let token_accounting = get(v, "methodology.token_accounting").and_then(Value::as_str);
    if let Some(m) = metrics {
        if m.is_empty() && status.is_some_and(|s| s != "NOT_RUN") {
            d("metrics", "is empty".into());
        }
        if !m.is_empty() && status == Some("NOT_RUN") {
            d("metrics", "NOT_RUN records carry no measurements".into());
        }
        for (k, x) in m {
            let field = format!("metrics.{k}");
            if !is_finite_number(x) {
                d(&field, "must be a finite number".into());
            }
            if let Some(defs) = definitions {
                if !defs.contains_key(k) {
                    d(
                        &format!("methodology.definitions.{k}"),
                        format!("missing definition for metric {k}"),
                    );
                }
            }
            if let Some(q) = percentile_of(k) {
                let need = min_samples_for(q);
                match samples {
                    Ok(Some(n)) if n < need => d(
                        &field,
                        format!("p{q} needs at least {need} samples, record has {n}"),
                    ),
                    Ok(None) => d(
                        &field,
                        format!("p{q} needs a recorded sample count (methodology.samples is not_recorded)"),
                    ),
                    _ => {}
                }
            }
            if token_accounting == Some("events_not_tokens") && claims_tokens(k) {
                d(
                    &field,
                    "token metric with token_accounting events_not_tokens (stream events are not tokens)"
                        .into(),
                );
            }
        }
    }

    if let Ok(Some(n)) = samples {
        if status == Some("NOT_RUN") {
            if n != 0 {
                d(
                    "methodology.samples",
                    "NOT_RUN records have 0 samples".into(),
                );
            }
        } else if n < 1 {
            d("methodology.samples", "must be at least 1".into());
        }
    }

    match v.get("evidence") {
        None => d("evidence", "missing".into()),
        Some(Value::Array(a)) => {
            if a.is_empty() && status.is_some_and(|s| s != "NOT_RUN") {
                d(
                    "evidence",
                    "is empty (only NOT_RUN records may have no evidence)".into(),
                );
            }
            if !a.is_empty() && status == Some("NOT_RUN") {
                d("evidence", "NOT_RUN records carry no evidence".into());
            }
            for (i, x) in a.iter().enumerate() {
                let field = format!("evidence[{i}]");
                match x.as_str() {
                    None => d(&field, "must be a string".into()),
                    Some(e) if is_placeholder(e) => {}
                    Some(e) if !is_inside_relative(e) => d(
                        &field,
                        format!("\"{e}\" must be a relative path inside the repository"),
                    ),
                    Some(e) if !root.join(e).exists() => {
                        d(&field, format!("path does not exist: {e}"))
                    }
                    Some(_) => {}
                }
            }
        }
        Some(_) => d("evidence", "must be an array of paths".into()),
    }

    if status == Some("VERIFIED") && dirty != Some(false) {
        d("source.dirty", "VERIFIED requires dirty == false".into());
    }

    // Placeholders and not_recorded anywhere in the document.
    let mut strings = Vec::new();
    collect_strings(v, String::new(), &mut strings);
    for (field, s) in strings {
        if is_placeholder(&s) {
            d(&field, format!("placeholder value \"{s}\""));
        } else if s == NOT_RECORDED {
            if !NOT_RECORDED_FIELDS.contains(&field.as_str()) {
                d(&field, "not_recorded is not allowed in this field".into());
            } else if status.is_some() && !lenient {
                d(
                    &field,
                    "not_recorded is only allowed in HISTORICAL or SUPERSEDED records".into(),
                );
            } else if !limitations.iter().any(|l| l.contains(field.as_str())) {
                d(
                    &field,
                    "not_recorded must be explained by a limitations entry naming this field"
                        .into(),
                );
            }
        }
    }
}

/// Check one SHA256SUMS bundle directory; returns the number of files whose digest was checked.
pub fn check_bundle(root: &Path, dir: &Path, out: &mut Vec<Defect>) -> usize {
    let display_dir = display_path(root, dir);
    let sums_path = dir.join("SHA256SUMS");
    let sums_display = format!("{display_dir}/SHA256SUMS");
    let text = match fs::read_to_string(&sums_path) {
        Ok(t) => t,
        Err(e) => {
            push(out, &sums_display, "(file)", format!("cannot read: {e}"));
            return 0;
        }
    };
    let mut listed = Vec::new();
    let mut checked = 0;
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let parsed = line.split_once(char::is_whitespace).and_then(|(h, rest)| {
            let name = rest.trim_start().trim_start_matches('*');
            let ok = h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit());
            (ok && !name.is_empty() && is_inside_relative(name)).then_some((h, name))
        });
        let Some((hash, name)) = parsed else {
            push(
                out,
                &sums_display,
                &format!("line {}", i + 1),
                "malformed (expected <64 hex>  <relative name>)".into(),
            );
            continue;
        };
        listed.push(name.to_string());
        let file_display = format!("{display_dir}/{name}");
        match fs::read(dir.join(name)) {
            Err(_) => push(
                out,
                &file_display,
                "SHA256SUMS",
                "listed file is missing".into(),
            ),
            Ok(bytes) => {
                checked += 1;
                let actual = hex(&Sha256::digest(&bytes));
                if !actual.eq_ignore_ascii_case(hash) {
                    push(
                        out,
                        &file_display,
                        "sha256",
                        "does not match SHA256SUMS".into(),
                    );
                }
            }
        }
    }
    if let Ok(entries) = fs::read_dir(dir) {
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "SHA256SUMS")
            .collect();
        names.sort();
        for n in names {
            if !listed.contains(&n) {
                push(
                    out,
                    &format!("{display_dir}/{n}"),
                    "SHA256SUMS",
                    "file is not listed".into(),
                );
            }
        }
    }
    checked
}

/// Checks a pre-contract bundle JSON shares with records: no placeholders, full commit ids.
fn check_legacy_bundle_json(path: &str, v: &Value, out: &mut Vec<Defect>) {
    let mut strings = Vec::new();
    collect_strings(v, String::new(), &mut strings);
    for (field, s) in strings {
        if is_placeholder(&s) {
            push(out, path, &field, format!("placeholder value \"{s}\""));
        } else if field.ends_with("_git_sha") && !is_commit(&s) {
            push(
                out,
                path,
                &field,
                format!("\"{s}\" is not a 40-character lowercase hex commit id"),
            );
        }
    }
}

fn check_enum(v: &Value, field: &str, allowed: &[&str], d: &mut impl FnMut(&str, String)) {
    match get(v, field) {
        None => d(field, "missing".into()),
        Some(Value::String(s)) if s == NOT_RECORDED || allowed.contains(&s.as_str()) => {}
        Some(Value::String(s)) if is_placeholder(s) => {}
        Some(Value::String(s)) => d(
            field,
            format!("\"{s}\" is not one of {}", allowed.join(", ")),
        ),
        Some(_) => d(field, "must be a string".into()),
    }
}

/// `pNN` key segment (1..=99), for example `ttft_ms_p95` -> 95.
pub fn percentile_of(key: &str) -> Option<u64> {
    key.split('_').find_map(|seg| {
        let digits = seg.strip_prefix('p')?;
        if digits.is_empty() || digits.len() > 2 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let q: u64 = digits.parse().ok()?;
        (1..=99).contains(&q).then_some(q)
    })
}

/// Smallest sample count for which the nearest-rank `pq` is not forced to be the maximum.
pub fn min_samples_for(q: u64) -> u64 {
    let tail = 100 - q;
    100_u64.div_ceil(tail)
}

/// Key segments that claim token counts or token timing (`tok`, `tokens`, `itl`); `first_token`
/// alone is allowed because time to the first content event is still measurable.
fn claims_tokens(key: &str) -> bool {
    key.split('_')
        .any(|s| matches!(s, "tok" | "tokens" | "itl"))
}

fn is_placeholder(s: &str) -> bool {
    PLACEHOLDERS.contains(&s.trim().to_ascii_lowercase().as_str())
}

/// 40 lowercase hex characters.
pub fn is_commit(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_finite_number(v: &Value) -> bool {
    v.as_f64().is_some_and(f64::is_finite)
}

fn is_inside_relative(p: &str) -> bool {
    let path = Path::new(p);
    !p.is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn get<'a>(v: &'a Value, dotted: &str) -> Option<&'a Value> {
    dotted.split('.').try_fold(v, |cur, k| cur.get(k))
}

fn collect_strings(v: &Value, prefix: String, out: &mut Vec<(String, String)>) {
    match v {
        Value::String(s) => out.push((prefix, s.clone())),
        Value::Array(a) => {
            for (i, x) in a.iter().enumerate() {
                collect_strings(x, format!("{prefix}[{i}]"), out);
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                collect_strings(x, p, out);
            }
        }
        _ => {}
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let p = e.path();
        let name = e.file_name();
        if name == ".git" || name == "target" {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Path relative to `root` with `/` separators (falls back to the path as given).
pub fn display_path(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    rel.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn push(out: &mut Vec<Defect>, path: &str, field: &str, reason: String) {
    out.push(Defect {
        path: path.to_string(),
        field: field.to_string(),
        reason,
    });
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base() -> Value {
        json!({
            "schema": RECORD_SCHEMA,
            "status": "EXPERIMENTAL",
            "source": {"repo": "aien-spark/benchmarks", "commit": "0123456789abcdef0123456789abcdef01234567", "dirty": false},
            "build": {"profile": "release", "rustc": "rustc 1.0.0"},
            "workload": {"name": "w", "concurrency": 1},
            "hardware": {"host": "h", "cpu": "c", "gpu": "none_detected"},
            "backend": {"engine": "e", "model": "m", "precision": "unlabeled", "endpoint": "in_process"},
            "methodology": {"clock": "monotonic", "definitions": {"x_ms": "d"}, "token_accounting": "backend_usage", "samples": 3},
            "metrics": {"x_ms": 1.5},
            "evidence": ["Cargo.toml"],
            "limitations": []
        })
    }

    fn run(v: &Value) -> Vec<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut out = Vec::new();
        check_record(root, "r.json", v, &mut out);
        out.sort();
        out.dedup();
        out.iter().map(|d| d.to_string()).collect()
    }

    #[test]
    fn base_record_is_clean() {
        assert_eq!(run(&base()), Vec::<String>::new());
    }

    #[test]
    fn percentile_minimums() {
        assert_eq!(percentile_of("ttft_ms_p95"), Some(95));
        assert_eq!(percentile_of("p50"), Some(50));
        assert_eq!(percentile_of("step_us_p999"), None);
        assert_eq!(percentile_of("prefix_tokens"), None);
        assert_eq!(min_samples_for(50), 2);
        assert_eq!(min_samples_for(90), 10);
        assert_eq!(min_samples_for(95), 20);
        assert_eq!(min_samples_for(99), 100);
    }

    #[test]
    fn nested_placeholder_and_case() {
        let mut v = base();
        v["limitations"] = json!(["  N/a "]);
        assert_eq!(
            run(&v),
            vec!["r.json: limitations[0]: placeholder value \"  N/a \""]
        );
    }

    #[test]
    fn not_run_rules() {
        let mut v = base();
        v["status"] = json!("NOT_RUN");
        assert_eq!(
            run(&v),
            vec![
                "r.json: evidence: NOT_RUN records carry no evidence",
                "r.json: methodology.samples: NOT_RUN records have 0 samples",
                "r.json: metrics: NOT_RUN records carry no measurements",
            ]
        );
        v["evidence"] = json!([]);
        v["metrics"] = json!({});
        v["methodology"]["samples"] = json!(0);
        assert_eq!(run(&v), Vec::<String>::new());
    }

    #[test]
    fn evidence_must_stay_inside_the_repository() {
        let mut v = base();
        v["evidence"] = json!(["../outside.json", "/etc/hostname"]);
        assert_eq!(
            run(&v),
            vec![
                "r.json: evidence[0]: \"../outside.json\" must be a relative path inside the repository",
                "r.json: evidence[1]: \"/etc/hostname\" must be a relative path inside the repository",
            ]
        );
    }

    #[test]
    fn not_recorded_needs_lenient_status_and_a_limitation() {
        let mut v = base();
        v["build"]["rustc"] = json!(NOT_RECORDED);
        assert_eq!(
            run(&v),
            vec!["r.json: build.rustc: not_recorded is only allowed in HISTORICAL or SUPERSEDED records"]
        );
        v["status"] = json!("HISTORICAL");
        assert_eq!(
            run(&v),
            vec!["r.json: build.rustc: not_recorded must be explained by a limitations entry naming this field"]
        );
        v["limitations"] = json!(["build.rustc: the run did not record the compiler"]);
        assert_eq!(run(&v), Vec::<String>::new());
        v["workload"]["name"] = json!(NOT_RECORDED);
        assert_eq!(
            run(&v),
            vec!["r.json: workload.name: not_recorded is not allowed in this field"]
        );
    }

    #[test]
    fn metric_must_be_number_and_defined() {
        let mut v = base();
        v["metrics"] = json!({"x_ms": "1.5", "y_ms": 2});
        assert_eq!(
            run(&v),
            vec![
                "r.json: methodology.definitions.y_ms: missing definition for metric y_ms",
                "r.json: metrics.x_ms: must be a finite number",
            ]
        );
    }
}
