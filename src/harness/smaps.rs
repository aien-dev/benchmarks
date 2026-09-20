use std::fs;

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessMemorySnapshot {
    pub rss_kb: u64,
    pub pss_kb: u64,
    pub pss_dirty_kb: u64,
    pub private_dirty_kb: u64,
    pub shared_clean_kb: u64,
}

impl ProcessMemorySnapshot {
    pub fn rss_mb(&self) -> f64 {
        self.rss_kb as f64 / 1024.0
    }
    pub fn pss_mb(&self) -> f64 {
        self.pss_kb as f64 / 1024.0
    }
}

pub fn read_smaps_rollup(pid: u32) -> Option<ProcessMemorySnapshot> {
    let path = format!("/proc/{}/smaps_rollup", pid);
    let content = fs::read_to_string(&path).ok()?;
    let mut snap = ProcessMemorySnapshot::default();
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Rss:") {
            snap.rss_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("Pss:") {
            snap.pss_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("Pss_Dirty:") {
            snap.pss_dirty_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("Private_Dirty:") {
            snap.private_dirty_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("Shared_Clean:") {
            snap.shared_clean_kb = parse_kb(rest);
        }
    }
    Some(snap)
}

fn parse_kb(s: &str) -> u64 {
    s.split_whitespace()
        .next()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
}
