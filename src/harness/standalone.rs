use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub struct RustStandaloneServer {
    pub port: u16,
    running: Arc<AtomicBool>,
}

impl RustStandaloneServer {
    pub fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind ephemeral port: {}", e))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        thread::spawn(move || {
            while running_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        thread::spawn(move || {
                            handle_client(stream);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(_) => break,
                }
            }
        });

        // Verify listener responds
        let start = Instant::now();
        let mut ready = false;
        while start.elapsed() < Duration::from_secs(2) {
            if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
                ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        if !ready {
            return Err("Rust standalone server failed to respond on port".to_string());
        }

        Ok(Self { port, running })
    }
}

impl Drop for RustStandaloneServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

fn handle_client(mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let mut buf = [0u8; 1024];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let req = String::from_utf8_lossy(&buf[..n]);

    let (status, body) = if req.starts_with("GET /api/status") || req.starts_with("GET /health") {
        (
            "200 OK",
            r#"{"status":"ok","engine":"rust-standalone","version":"0.1"}"#.to_string(),
        )
    } else if req.starts_with("GET /api/query") {
        ("200 OK", r#"{"id":1,"canonical_name":"benchmark_reference_entity","content":"Verified cryptographic token record for baseline testing"}"#.to_string())
    } else if req.starts_with("GET /api/vector") {
        let v1 = [0.035f32; 768];
        let v2 = [0.042f32; 768];
        let dot: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
        (
            "200 OK",
            format!(r#"{{"similarity":{:.5},"dimensions":768}}"#, dot),
        )
    } else {
        ("404 Not Found", r#"{"error":"not found"}"#.to_string())
    };

    let resp = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        body.len(),
        body
    );

    let _ = stream.write_all(resp.as_bytes());
}

pub struct PythonBaselineServer {
    pub port: u16,
    pub pid: u32,
    child: Child,
}

impl PythonBaselineServer {
    pub fn start(script_path: &str) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        drop(listener);

        let mut child = Command::new("python3")
            .arg(script_path)
            .arg(port.to_string())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn python3 {}: {}", script_path, e))?;

        let pid = child.id();

        // Poll until port opens
        let start = Instant::now();
        let mut ready = false;
        while start.elapsed() < Duration::from_secs(4) {
            if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
                ready = true;
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }

        if !ready {
            let _ = child.kill();
            return Err(
                "Python baseline microservice failed to bind port within 4 seconds".to_string(),
            );
        }

        Ok(Self { port, pid, child })
    }
}

impl Drop for PythonBaselineServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
