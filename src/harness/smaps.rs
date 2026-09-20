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


pub fn get_process_tree_pids(root_pid: u32) -> Vec<u32> {
    let mut pids = vec![root_pid];
    let mut queue = vec![root_pid];
    while let Some(parent) = queue.pop() {
        let task_dir = format!("/proc/{}/task", parent);
        if let Ok(entries) = fs::read_dir(task_dir) {
            for entry in entries.flatten() {
                let children_path = entry.path().join("children");
                if let Ok(content) = fs::read_to_string(children_path) {
                    for child_str in content.split_whitespace() {
                        if let Ok(child_pid) = child_str.parse::<u32>() {
                            if !pids.contains(&child_pid) {
                                pids.push(child_pid);
                                queue.push(child_pid);
                            }
                        }
                    }
                }
            }
        }
    }
    pids
}

pub fn read_process_tree_smaps(root_pid: u32) -> Option<ProcessMemorySnapshot> {
    let pids = get_process_tree_pids(root_pid);
    let mut total = ProcessMemorySnapshot::default();
    let mut found_any = false;
    for pid in pids {
        if let Some(snap) = read_smaps_rollup(pid) {
            total.rss_kb += snap.rss_kb;
            total.pss_kb += snap.pss_kb;
            total.pss_dirty_kb += snap.pss_dirty_kb;
            total.private_dirty_kb += snap.private_dirty_kb;
            total.shared_clean_kb += snap.shared_clean_kb;
            found_any = true;
        }
    }
    if found_any {
        Some(total)
    } else {
        None
    }
}
