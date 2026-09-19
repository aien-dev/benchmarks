use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HttpBenchResult {
    pub endpoint: String,
    pub total_requests: usize,
    pub successful: usize,
    pub failed: usize,
    pub total_elapsed_secs: f64,
    pub requests_per_sec: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
}

fn send_single_request(host: &str, port: u16, path: &str, auth_token: Option<&str>) -> Result<f64, String> {
    let start = Instant::now();
    let addr = format!("{}:{}", host, port);
    let mut stream = TcpStream::connect(&addr).map_err(|e| format!("Connect failed: {}", e))?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(Duration::from_secs(5))).map_err(|e| e.to_string())?;

    let mut req = format!(
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nUser-Agent: aien-benchmarks/0.1\r\n",
        path, host, port
    );
    if let Some(tok) = auth_token {
        req.push_str(&format!("Authorization: Bearer {}\r\n", tok));
    }
    req.push_str("\r\n");

    stream.write_all(req.as_bytes()).map_err(|e| format!("Write error: {}", e))?;

    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).map_err(|e| format!("Read error: {}", e))?;
    if n == 0 {
        return Err("Empty response".to_string());
    }

    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
    Ok(elapsed)
}

pub fn run_http_benchmark(
    host: &str,
    port: u16,
    path: &str,
    auth_token: Option<&str>,
    concurrency: usize,
    total_requests: usize,
    warmup: usize,
) -> Result<HttpBenchResult, String> {
    // 1. Warmup
    for _ in 0..warmup {
        let _ = send_single_request(host, port, path, auth_token);
    }

    // 2. Concurrency execution
    let requests_per_worker = total_requests / concurrency.max(1);
    let all_latencies = Arc::new(Mutex::new(Vec::with_capacity(total_requests)));
    let successful_count = Arc::new(Mutex::new(0usize));
    let failed_count = Arc::new(Mutex::new(0usize));

    let overall_start = Instant::now();
    let mut handles = Vec::with_capacity(concurrency);

    for _ in 0..concurrency {
        let host_c = host.to_string();
        let path_c = path.to_string();
        let tok_c = auth_token.map(|s| s.to_string());
        let latencies_arc = Arc::clone(&all_latencies);
        let success_arc = Arc::clone(&successful_count);
        let failed_arc = Arc::clone(&failed_count);

        handles.push(thread::spawn(move || {
            let mut local_samples = Vec::with_capacity(requests_per_worker);
            let mut local_succ = 0;
            let mut local_fail = 0;

            for _ in 0..requests_per_worker {
                match send_single_request(&host_c, port, &path_c, tok_c.as_deref()) {
                    Ok(lat) => {
                        local_samples.push(lat);
                        local_succ += 1;
                    }
                    Err(_) => {
                        local_fail += 1;
                    }
                }
            }

            {
                let mut l = latencies_arc.lock().unwrap();
                l.extend(local_samples);
            }
            {
                let mut s = success_arc.lock().unwrap();
                *s += local_succ;
            }
            {
                let mut f = failed_arc.lock().unwrap();
                *f += local_fail;
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    let overall_elapsed = overall_start.elapsed().as_secs_f64();
    let mut latencies = all_latencies.lock().unwrap().clone();
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let successful = *successful_count.lock().unwrap();
    let failed = *failed_count.lock().unwrap();

    if latencies.is_empty() {
        return Err("All benchmark requests failed".to_string());
    }

    let count = latencies.len();
    let p50 = latencies[count * 50 / 100];
    let p95 = latencies[count * 95 / 100];
    let p99 = latencies[(count * 99 / 100).min(count - 1)];
    let min_val = latencies[0];
    let max_val = latencies[count - 1];
    let req_per_sec = (successful as f64) / overall_elapsed.max(0.0001);

    Ok(HttpBenchResult {
        endpoint: format!("http://{}:{}{}", host, port, path),
        total_requests: count,
        successful,
        failed,
        total_elapsed_secs: overall_elapsed,
        requests_per_sec: req_per_sec,
        p50_ms: p50,
        p95_ms: p95,
        p99_ms: p99,
        min_ms: min_val,
        max_ms: max_val,
    })
}
