//! Run one implementation process once and capture everything it produced.
use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Executed {
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub report: Option<Vec<u8>>,
    pub bitmap: Option<Vec<u8>>,
    pub wall_s: f64,
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Execute `imp` with the implementation CLI of the contract.
/// `audit = Some(k)` runs audit mode with k passes, otherwise timed mode.
pub fn execute(imp: &Path, limit: u64, min_seconds: f64, audit: Option<u32>) -> io::Result<Executed> {
    let dir = std::env::temp_dir().join(format!(
        "prime-race-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir)?;
    let bm = dir.join("bitmap.bin");
    let rp = dir.join("report.json");
    let mut tries = 0;
    let (out, wall_s) = loop {
        let mut cmd = Command::new(imp);
        cmd.arg("--limit").arg(limit.to_string());
        match audit {
            Some(k) => {
                cmd.arg("--audit-passes").arg(k.to_string());
            }
            None => {
                cmd.arg("--min-seconds").arg(min_seconds.to_string());
            }
        }
        cmd.arg("--bitmap-out").arg(&bm).arg("--report-out").arg(&rp);
        cmd.stdin(Stdio::null());
        let t = Instant::now();
        match cmd.output() {
            Ok(o) => break (o, t.elapsed().as_secs_f64()),
            // A freshly written script can briefly be busy; retry only that case.
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&dir);
                return Err(e);
            }
        }
    };
    let ex = Executed {
        exit_code: out.status.code(),
        stdout: out.stdout,
        stderr: out.stderr,
        report: fs::read(&rp).ok(),
        bitmap: fs::read(&bm).ok(),
        wall_s,
    };
    let _ = fs::remove_dir_all(&dir);
    Ok(ex)
}
