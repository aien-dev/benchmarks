//! Strict validation of one implementation run against the canonical result.
use super::exec::Executed;
use super::reference::{self, Diff};
use super::REPORT_SCHEMA;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub label: String,
    pub passes: u64,
    pub elapsed_text: String,
    pub elapsed: f64,
    pub threads: u64,
    pub algorithm: String,
    pub faithful: String,
    pub bits: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tags {
    pub algorithm: String,
    pub faithful: String,
    pub bits: u64,
    pub threads: Option<u64>,
}

impl Tags {
    /// "algorithm=base,faithful=yes,bits=1[,threads=1]"
    pub fn parse(text: &str) -> Result<Tags, String> {
        let (mut a, mut f, mut b, mut t) = (None, None, None, None);
        for kv in text.split(',') {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("bad tag {kv}"))?;
            match k {
                "algorithm" => a = Some(v.to_string()),
                "faithful" => f = Some(v.to_string()),
                "bits" => b = Some(v.parse::<u64>().map_err(|_| format!("bad bits {v}"))?),
                "threads" => t = Some(v.parse::<u64>().map_err(|_| format!("bad threads {v}"))?),
                _ => return Err(format!("unknown tag {k}")),
            }
        }
        Ok(Tags {
            algorithm: a.ok_or("missing algorithm")?,
            faithful: f.ok_or("missing faithful")?,
            bits: b.ok_or("missing bits")?,
            threads: t,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Expect {
    /// Implementation name N: the stdout label must be `aien-N`, report impl must be N.
    pub name: String,
    pub limit: u64,
    pub min_seconds: f64,
    pub audit: Option<u32>,
    pub tags: Tags,
}

impl Expect {
    pub fn line_label(&self) -> String {
        format!("aien-{}", self.name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reject {
    Spawn(String),
    ExitCode(Option<i32>),
    NoStdoutLine,
    AuditStdoutLine,
    MultipleStdoutLines(usize),
    MalformedLine(String),
    ReportMissing,
    BadReport(String),
    StatusNotComplete(String),
    LabelMismatch(String),
    LineReportMismatch(String),
    PassesZero,
    ElapsedBelowMin { elapsed: f64, min: f64 },
    AuditPassCount { got: u64, want: u32 },
    BitmapMissing,
    BitmapLength { got: usize, want: usize },
    BitmapDiff(Diff),
    AuditPassDiff { pass: usize, diff: Diff },
}

impl Reject {
    pub fn kind(&self) -> &'static str {
        match self {
            Reject::Spawn(_) => "spawn_error",
            Reject::ExitCode(_) => "exit_code",
            Reject::NoStdoutLine => "no_stdout_line",
            Reject::AuditStdoutLine => "audit_stdout_line",
            Reject::MultipleStdoutLines(_) => "multiple_stdout_lines",
            Reject::MalformedLine(_) => "malformed_line",
            Reject::ReportMissing => "report_missing",
            Reject::BadReport(_) => "bad_report",
            Reject::StatusNotComplete(_) => "status_not_complete",
            Reject::LabelMismatch(_) => "label_mismatch",
            Reject::LineReportMismatch(_) => "line_report_mismatch",
            Reject::PassesZero => "passes_zero",
            Reject::ElapsedBelowMin { .. } => "elapsed_below_min",
            Reject::AuditPassCount { .. } => "audit_pass_count",
            Reject::BitmapMissing => "bitmap_missing",
            Reject::BitmapLength { .. } => "bitmap_length",
            Reject::BitmapDiff(_) => "bitmap_diff",
            Reject::AuditPassDiff { .. } => "audit_pass_diff",
        }
    }
}

impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.kind())?;
        match self {
            Reject::Spawn(e) | Reject::MalformedLine(e) | Reject::BadReport(e) | Reject::LabelMismatch(e)
            | Reject::LineReportMismatch(e) | Reject::StatusNotComplete(e) => write!(f, "{e}"),
            Reject::ExitCode(c) => write!(f, "exit code {c:?}, expected 0"),
            Reject::NoStdoutLine => write!(f, "no result line on stdout"),
            Reject::AuditStdoutLine => write!(f, "audit mode printed a stdout line; audit runs make no timing claim"),
            Reject::MultipleStdoutLines(n) => write!(f, "{n} lines on stdout, expected exactly one"),
            Reject::ReportMissing => write!(f, "report file not written"),
            Reject::PassesZero => write!(f, "passes is 0"),
            Reject::ElapsedBelowMin { elapsed, min } => write!(f, "elapsed {elapsed} is below min_seconds {min}"),
            Reject::AuditPassCount { got, want } => write!(f, "audit passes {got}, wanted {want}"),
            Reject::BitmapMissing => write!(f, "bitmap file not written"),
            Reject::BitmapLength { got, want } => write!(f, "bitmap is {got} bytes, expected {want}"),
            Reject::BitmapDiff(d) => write!(f, "{d}"),
            Reject::AuditPassDiff { pass, diff } => write!(f, "audit pass {pass}: {diff}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Validated {
    pub passes: u64,
    pub elapsed: f64,
    /// The upstream line; `None` in audit mode, which prints no line.
    pub line: Option<Line>,
    pub report: Value,
}

/// Strict parse of `<label>;<passes>;<elapsed %.6f>;<threads>;algorithm=..,faithful=..,bits=..`.
pub fn parse_line(s: &str) -> Result<Line, String> {
    let parts: Vec<&str> = s.split(';').collect();
    if parts.len() != 5 {
        return Err(format!("expected 5 ';' fields, found {}", parts.len()));
    }
    let label = parts[0];
    if label.is_empty() || label.chars().any(|c| c.is_whitespace() || c == ',' || c == '=') {
        return Err("bad label".into());
    }
    let uint = |t: &str, what: &str| -> Result<u64, String> {
        if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) || (t.len() > 1 && t.starts_with('0')) {
            return Err(format!("bad {what} '{t}'"));
        }
        t.parse::<u64>().map_err(|_| format!("bad {what} '{t}'"))
    };
    let passes = uint(parts[1], "passes")?;
    let el = parts[2];
    let ok = match el.split_once('.') {
        Some((i, d)) => {
            !i.is_empty() && i.bytes().all(|b| b.is_ascii_digit()) && d.len() == 6 && d.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    };
    if !ok {
        return Err(format!("elapsed '{el}' is not %.6f"));
    }
    let elapsed: f64 = el.parse().map_err(|_| "bad elapsed".to_string())?;
    let threads = uint(parts[3], "threads")?;
    let tags: Vec<&str> = parts[4].split(',').collect();
    if tags.len() != 3 {
        return Err("tags must be algorithm,faithful,bits".into());
    }
    let tag = |t: &str, key: &str| -> Result<String, String> {
        t.strip_prefix(&format!("{key}="))
            .filter(|v| !v.is_empty() && !v.contains('='))
            .map(String::from)
            .ok_or_else(|| format!("expected tag {key}=.. got '{t}'"))
    };
    let algorithm = tag(tags[0], "algorithm")?;
    let faithful = tag(tags[1], "faithful")?;
    if faithful != "yes" && faithful != "no" {
        return Err(format!("faithful must be yes or no, got '{faithful}'"));
    }
    let bits = uint(&tag(tags[2], "bits")?, "bits")?;
    Ok(Line {
        label: label.to_string(),
        passes,
        elapsed_text: el.to_string(),
        elapsed,
        threads,
        algorithm,
        faithful,
        bits,
    })
}

fn rs(v: &Value, k: &str) -> Result<String, Reject> {
    v.get(k)
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| Reject::BadReport(format!("field {k} missing or not a string")))
}
fn ru(v: &Value, k: &str) -> Result<u64, Reject> {
    v.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| Reject::BadReport(format!("field {k} missing or not an unsigned integer")))
}
fn rf(v: &Value, k: &str) -> Result<f64, Reject> {
    v.get(k)
        .and_then(Value::as_f64)
        .ok_or_else(|| Reject::BadReport(format!("field {k} missing or not a number")))
}
fn rfaithful(v: &Value) -> Result<String, Reject> {
    match v.get("faithful") {
        Some(Value::Bool(b)) => Ok(if *b { "yes" } else { "no" }.into()),
        Some(Value::String(s)) => Ok(s.clone()),
        _ => Err(Reject::BadReport("field faithful missing".into())),
    }
}

fn mismatch<T: std::fmt::Debug + PartialEq>(what: &str, a: T, b: T) -> Result<(), Reject> {
    if a == b {
        Ok(())
    } else {
        Err(Reject::LineReportMismatch(format!("{what}: line {a:?}, report {b:?}")))
    }
}

pub fn validate_run(exp: &Expect, ex: &Executed) -> Result<Validated, Reject> {
    if ex.exit_code != Some(0) {
        return Err(Reject::ExitCode(ex.exit_code));
    }
    let text = std::str::from_utf8(&ex.stdout).map_err(|_| Reject::MalformedLine("stdout is not UTF-8".into()))?;
    // Timed mode prints exactly one upstream line. Audit mode makes no timing claim and must
    // print nothing on stdout; its labels are checked from the report alone.
    let line = match exp.audit {
        Some(_) => {
            if !text.is_empty() {
                return Err(Reject::AuditStdoutLine);
            }
            None
        }
        None => {
            if text.is_empty() {
                return Err(Reject::NoStdoutLine);
            }
            let body = text.strip_suffix('\n').unwrap_or(text);
            let n = body.split('\n').count();
            if n != 1 {
                return Err(Reject::MultipleStdoutLines(n));
            }
            Some(parse_line(body).map_err(Reject::MalformedLine)?)
        }
    };

    let raw = ex.report.as_ref().ok_or(Reject::ReportMissing)?;
    let rep: Value = serde_json::from_slice(raw).map_err(|e| Reject::BadReport(format!("not JSON: {e}")))?;
    if rs(&rep, "schema")? != REPORT_SCHEMA {
        return Err(Reject::BadReport("wrong schema".into()));
    }
    let status = rs(&rep, "status")?;
    if status != "COMPLETE" {
        return Err(Reject::StatusNotComplete(format!("status {status}")));
    }
    let r_impl = rs(&rep, "impl")?;
    let r_label = rs(&rep, "label")?;
    let r_alg = rs(&rep, "algorithm")?;
    let r_faith = rfaithful(&rep)?;
    let r_bits = ru(&rep, "bits")?;
    let r_threads = ru(&rep, "threads")?;
    let r_limit = ru(&rep, "limit")?;
    let r_mode = rs(&rep, "mode")?;
    let r_passes = ru(&rep, "passes")?;
    let r_elapsed = rf(&rep, "elapsed_s")?;
    rs(&rep, "environment")?;

    // Declared expectations, checked against the report.
    if r_impl != exp.name {
        return Err(Reject::LabelMismatch(format!("report impl '{r_impl}', expected '{}'", exp.name)));
    }
    if r_label != exp.line_label() {
        return Err(Reject::LabelMismatch(format!("report label '{r_label}', expected '{}'", exp.line_label())));
    }
    let t = &exp.tags;
    if r_alg != t.algorithm || r_faith != t.faithful || r_bits != t.bits {
        return Err(Reject::LabelMismatch(format!(
            "tags algorithm={r_alg},faithful={r_faith},bits={r_bits} differ from declared algorithm={},faithful={},bits={}",
            t.algorithm, t.faithful, t.bits
        )));
    }
    if let Some(th) = t.threads {
        if r_threads != th {
            return Err(Reject::LabelMismatch(format!("threads {r_threads} differ from declared {th}")));
        }
    }
    let want_mode = if exp.audit.is_some() { "audit" } else { "timed" };
    if r_mode != want_mode {
        return Err(Reject::BadReport(format!("mode '{r_mode}', expected '{want_mode}'")));
    }
    if r_limit != exp.limit {
        return Err(Reject::BadReport(format!("limit {r_limit}, expected {}", exp.limit)));
    }

    // The upstream line must equal the report field for field.
    if let Some(line) = &line {
        if line.label != r_label {
            return Err(Reject::LabelMismatch(format!("line label '{}' differs from report label '{r_label}'", line.label)));
        }
        if line.algorithm != t.algorithm || line.faithful != t.faithful || line.bits != t.bits {
            return Err(Reject::LabelMismatch(format!(
                "line tags algorithm={},faithful={},bits={} differ from declared",
                line.algorithm, line.faithful, line.bits
            )));
        }
        mismatch("passes", line.passes, r_passes)?;
        mismatch("threads", line.threads, r_threads)?;
        mismatch("algorithm", line.algorithm.as_str(), r_alg.as_str())?;
        mismatch("faithful", line.faithful.as_str(), r_faith.as_str())?;
        mismatch("bits", line.bits, r_bits)?;
        if (line.elapsed - r_elapsed).abs() > 5.1e-7 {
            return Err(Reject::LineReportMismatch(format!("elapsed: line {}, report {r_elapsed}", line.elapsed_text)));
        }
    }

    if r_passes == 0 {
        return Err(Reject::PassesZero);
    }
    match exp.audit {
        None => {
            if !(r_elapsed >= exp.min_seconds) {
                return Err(Reject::ElapsedBelowMin { elapsed: r_elapsed, min: exp.min_seconds });
            }
            let ms = rf(&rep, "min_seconds")?;
            if (ms - exp.min_seconds).abs() > 1e-9 {
                return Err(Reject::BadReport(format!("report min_seconds {ms}, expected {}", exp.min_seconds)));
            }
        }
        Some(k) => {
            if r_passes != k as u64 {
                return Err(Reject::AuditPassCount { got: r_passes, want: k });
            }
        }
    }

    let bm = ex.bitmap.as_ref().ok_or(Reject::BitmapMissing)?;
    let want = reference::canonical_bytes(exp.limit);
    check_bitmap(&want, bm, exp)?;
    Ok(Validated { passes: r_passes, elapsed: r_elapsed, line, report: rep })
}

fn check_bitmap(want: &[u8], got: &[u8], exp: &Expect) -> Result<(), Reject> {
    let k = exp.audit.unwrap_or(1) as usize;
    if got.len() != want.len() * k {
        return Err(Reject::BitmapLength { got: got.len(), want: want.len() * k });
    }
    if want.is_empty() {
        return Ok(());
    }
    for (i, chunk) in got.chunks(want.len()).enumerate() {
        if let Some(d) = reference::first_difference(want, chunk, exp.limit) {
            return Err(if exp.audit.is_some() { Reject::AuditPassDiff { pass: i, diff: d } } else { Reject::BitmapDiff(d) });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_ok() {
        let l = parse_line("aien-cpu-c-base;123;5.000123;1;algorithm=base,faithful=yes,bits=1").unwrap();
        assert_eq!((l.passes, l.threads, l.bits), (123, 1, 1));
    }

    #[test]
    fn line_strict() {
        for bad in [
            "",
            "a;1;5.000000;1",
            "a;1;5;1;algorithm=base,faithful=yes,bits=1",
            "a;1;5.0;1;algorithm=base,faithful=yes,bits=1",
            "a;-1;5.000000;1;algorithm=base,faithful=yes,bits=1",
            "a;1;5.000000;1;algorithm=base,faithful=maybe,bits=1",
            "a;1;5.000000;1;faithful=yes,algorithm=base,bits=1",
            "a;1;5.000000;1;algorithm=base,faithful=yes",
            "a;1;5.000000;1;algorithm=base,faithful=yes,bits=1;x",
            " a;1;5.000000;1;algorithm=base,faithful=yes,bits=1",
        ] {
            assert!(parse_line(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn tags_parse() {
        let t = Tags::parse("algorithm=base,faithful=yes,bits=1,threads=1").unwrap();
        assert_eq!(t.threads, Some(1));
        assert!(Tags::parse("algorithm=base").is_err());
    }
}
