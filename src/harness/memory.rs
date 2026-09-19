use std::fs;
use std::process::Command;

pub fn read_process_rss_mb(pid: u32) -> Option<f64> {
    // 1. Check Linux /proc/<pid>/status
    let status_path = format!("/proc/{}/status", pid);
    if let Ok(content) = fs::read_to_string(&status_path) {
        for line in content.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<f64>() {
                        return Some(kb / 1024.0);
                    }
                }
            }
        }
    }

    // 2. Fallback to `ps -o rss= -p <pid>`
    if let Ok(output) = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
    {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(kb) = text.parse::<f64>() {
            return Some(kb / 1024.0);
        }
    }

    None
}

pub fn find_pid_by_pattern(pattern: &str) -> Option<u32> {
    // Try pgrep
    if let Ok(output) = Command::new("pgrep").args(["-f", pattern]).output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                if pid != std::process::id() {
                    return Some(pid);
                }
            }
        }
    }

    // Fallback: scan /proc
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if let Ok(pid) = name.to_string_lossy().parse::<u32>() {
                if pid == std::process::id() {
                    continue;
                }
                let cmdline_path = format!("/proc/{}/cmdline", pid);
                if let Ok(cmdline) = fs::read_to_string(&cmdline_path) {
                    if cmdline.contains(pattern) {
                        return Some(pid);
                    }
                }
            }
        }
    }

    None
}
