//! Evidence storage: content-addressed blobs and receipts (mode 0444, no overwrite).
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn sha256_file(path: &Path) -> io::Result<String> {
    Ok(sha256_hex(&fs::read(path)?))
}

fn write_new_ro(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).mode(0o444).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Store bytes as blobs/<sha256>; identical content is shared. Returns the digest.
pub fn store_blob(evidence: &Path, bytes: &[u8]) -> io::Result<String> {
    let sha = sha256_hex(bytes);
    let p = evidence.join("blobs").join(&sha);
    match write_new_ro(&p, bytes) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    Ok(sha)
}

/// Write the receipt as <sha256-of-bytes>.json. Refuses to overwrite.
pub fn write_receipt(evidence: &Path, receipt: &Value) -> io::Result<(PathBuf, String)> {
    let mut bytes = serde_json::to_vec_pretty(receipt).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let sha = sha256_hex(&bytes);
    let p = evidence.join(format!("{sha}.json"));
    write_new_ro(&p, &bytes)?;
    Ok((p, sha))
}

fn git_toplevel(dir: &Path) -> Option<PathBuf> {
    let o = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--show-toplevel"]).output().ok()?;
    if !o.status.success() {
        return None;
    }
    PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()).canonicalize().ok()
}

/// Resolve `dir` (which may not exist yet) and refuse anything inside this checkout.
pub fn check_evidence_dir(dir: &Path) -> Result<PathBuf, String> {
    let abs = if dir.is_absolute() { dir.to_path_buf() } else { std::env::current_dir().map_err(|e| e.to_string())?.join(dir) };
    let mut existing = abs.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = vec![];
    while !existing.exists() {
        rest.push(existing.file_name().ok_or("bad evidence dir")?);
        existing = existing.parent().ok_or("bad evidence dir")?;
    }
    let mut resolved = existing.canonicalize().map_err(|e| e.to_string())?;
    for r in rest.into_iter().rev() {
        resolved.push(r);
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().map_err(|e| e.to_string())?;
    if resolved.starts_with(&root) {
        return Err(format!("evidence dir {} is inside the benchmarks checkout", resolved.display()));
    }
    if let (Some(a), Some(b)) = (git_toplevel(existing), git_toplevel(&root)) {
        if a == b {
            return Err(format!("evidence dir {} is inside the benchmarks checkout", resolved.display()));
        }
    }
    Ok(resolved)
}

pub struct Stats {
    pub n: usize,
    pub median: f64,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub stddev: f64,
    pub cv: f64,
}

pub fn stats(samples: &[f64]) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = s.len();
    let mean = s.iter().sum::<f64>() / n as f64;
    let median = if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 };
    let var = if n > 1 { s.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0) } else { 0.0 };
    let stddev = var.sqrt();
    Some(Stats { n, median, min: s[0], max: s[n - 1], mean, stddev, cv: if mean != 0.0 { stddev / mean } else { 0.0 } })
}

impl Stats {
    pub fn to_json(&self) -> Value {
        json!({"n": self.n, "median": self.median, "min": self.min, "max": self.max,
               "mean": self.mean, "stddev": self.stddev, "cv": self.cv, "unit": "passes_per_second"})
    }
}

fn cmd_out(prog: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(prog).args(args).output().ok()?;
    if !o.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
}

pub fn loadavg() -> String {
    fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "unavailable".into())
}

pub fn governors() -> Vec<String> {
    let mut v: Vec<String> = vec![];
    if let Ok(rd) = fs::read_dir("/sys/devices/system/cpu") {
        for e in rd.flatten() {
            if let Ok(g) = fs::read_to_string(e.path().join("cpufreq/scaling_governor")) {
                let g = g.trim().to_string();
                if !v.contains(&g) {
                    v.push(g);
                }
            }
        }
    }
    v.sort();
    v
}

pub fn hardware() -> Value {
    let cpus: Vec<String> = cmd_out("lscpu", &[])
        .map(|t| {
            let mut v: Vec<String> = vec![];
            for l in t.lines() {
                if let Some(m) = l.strip_prefix("Model name:") {
                    let m = m.trim().to_string();
                    if !v.contains(&m) {
                        v.push(m);
                    }
                }
            }
            v
        })
        .unwrap_or_default();
    let gpu = cmd_out("nvidia-smi", &["--query-gpu=name,uuid,driver_version", "--format=csv,noheader"]);
    json!({"cpu_models": cpus, "nvidia_smi": gpu, "kernel": cmd_out("uname", &["-r"])})
}

pub fn runner_commit(arg: Option<&str>) -> String {
    if let Some(a) = arg {
        return a.to_string();
    }
    if let Some(c) = option_env!("PRIME_RACE_RUNNER_COMMIT") {
        return c.to_string();
    }
    cmd_out("git", &["-C", env!("CARGO_MANIFEST_DIR"), "rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_basic() {
        let s = stats(&[1.0, 2.0, 3.0, 4.0]).unwrap();
        assert_eq!(s.median, 2.5);
        assert_eq!((s.min, s.max, s.mean), (1.0, 4.0, 2.5));
        assert!((s.stddev - 1.2909944487358056).abs() < 1e-12);
    }

    #[test]
    fn evidence_dir_inside_checkout_refused() {
        let inside = Path::new(env!("CARGO_MANIFEST_DIR")).join("evidence-x");
        assert!(check_evidence_dir(&inside).is_err());
        assert!(check_evidence_dir(&std::env::temp_dir().join("pr-evidence-ok")).is_ok());
    }

    #[test]
    fn receipt_noclobber_and_mode() {
        let d = std::env::temp_dir().join(format!("pr-receipt-{}", std::process::id()));
        fs::create_dir_all(d.join("blobs")).unwrap();
        let v = json!({"a": 1});
        let (p, _) = write_receipt(&d, &v).unwrap();
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o444);
        assert!(write_receipt(&d, &v).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
