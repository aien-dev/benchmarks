use crate::models::{EnvironmentInfo, HardwareInfo};
use std::fs;
use std::process::Command;

pub fn detect_hardware() -> HardwareInfo {
    let mut system = "POSIX Compliant Workstation".to_string();
    let mut processor = std::env::consts::ARCH.to_string();
    let architecture = std::env::consts::ARCH.to_string();
    let mut memory_unified_gb = 16u32;
    let os = format!("{} {}", std::env::consts::OS, std::env::consts::FAMILY);

    // Try reading Linux /proc/cpuinfo
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
        for line in cpuinfo.lines() {
            if line.starts_with("model name") || line.starts_with("Processor") || line.starts_with("Hardware") {
                if let Some(val) = line.split(':').nth(1) {
                    processor = val.trim().to_string();
                    break;
                }
            }
        }
    }

    // Try reading Linux device model
    if let Ok(model) = fs::read_to_string("/proc/device-tree/model") {
        let trimmed = model.trim_matches('\0').trim();
        if !trimmed.is_empty() {
            system = trimmed.to_string();
        }
    } else if let Ok(hostname) = fs::read_to_string("/etc/hostname") {
        let trimmed = hostname.trim();
        if trimmed.contains("spark") {
            system = "NVIDIA DGX Spark".to_string();
        } else {
            system = trimmed.to_string();
        }
    }

    // Try reading Linux memory
    if let Ok(meminfo) = fs::read_to_string("/proc/meminfo") {
        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<u64>() {
                        memory_unified_gb = (kb / (1024 * 1024)) as u32;
                    }
                }
                break;
            }
        }
    }

    // Refine system strings if Grace Blackwell
    if processor.contains("Neoverse") || system.contains("DGX") || system.contains("Spark") {
        if system == "POSIX Compliant Workstation" {
            system = "NVIDIA DGX Spark".to_string();
        }
        if processor == "aarch64" || processor.is_empty() {
            processor = "NVIDIA Grace Blackwell (GB10, aarch64)".to_string();
        }
    }

    HardwareInfo {
        system,
        processor,
        architecture,
        memory_unified_gb,
        os,
    }
}

pub fn detect_environment(concurrency: u32, requests: u32, warmup: u32) -> EnvironmentInfo {
    let rustc_version = Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "rustc unknown".to_string());

    let python_version = Command::new("python3")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "python3 unknown".to_string());

    let kernel_version = Command::new("uname")
        .arg("-r")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "kernel unknown".to_string());

    let page_size_kb = get_page_size_kb();

    EnvironmentInfo {
        rustc_version,
        python_version,
        kernel_version,
        page_size_kb,
        concurrency_tested: concurrency,
        requests_per_endpoint: requests,
        warmup_requests: warmup,
    }
}

fn get_page_size_kb() -> u32 {
    if let Ok(output) = Command::new("getconf").arg("PAGESIZE").output() {
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Ok(bytes) = text.parse::<u32>() {
            if bytes > 0 {
                return bytes / 1024;
            }
        }
    }
    4
}
