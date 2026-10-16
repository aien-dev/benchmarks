//! Streaming benchmark for an OpenAI-compatible `/v1/chat/completions` endpoint.
//!
//! Measurement rules (issue aien-spark/aien-architecture#233, lane P0):
//! - One clock: `std::time::Instant` (monotonic). `request_start` is taken immediately
//!   before the request bytes are written to an already-connected socket. An event's arrival
//!   is the `Instant` taken immediately after the socket read that delivered its final byte
//!   returned (see `sse.rs`).
//! - One SSE event is not assumed to be one model token. Token metrics are reported only
//!   when the backend returns `usage.completion_tokens` for every successful request
//!   (`token_accounting: "backend_usage"`); otherwise only event metrics are reported
//!   (`token_accounting: "events_not_tokens"`).
//! - Percentiles come from N real observations with N recorded; p95 only when N >= 20.
//! - Read errors and timeouts are failures, never end-of-stream.

use super::sse::{HttpResponseParser, SseEvent, SseParser};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const CLOCK: &str = "std::time::Instant monotonic";
pub const RECORD_SCHEMA: &str = "aien-bench-record/1";
pub const SAMPLES_SCHEMA: &str = "aien-bench-samples/1";
pub const REPO: &str = "aien-spark/benchmarks";
/// p95 is reported only from at least this many observations.
pub const P95_MIN_SAMPLES: usize = 20;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
const ERROR_SNIPPET_CHARS: usize = 512;

/// Everything that identifies one streaming benchmark run. Labels (engine, model,
/// precision, context length) are reported exactly as configured.
#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub model: String,
    pub engine: String,
    pub precision: Option<String>,
    pub context_length: Option<u64>,
    pub max_tokens: u32,
    pub prompt: String,
    pub requests: u32,
    pub concurrency: u32,
    pub warmup: u32,
    pub request_usage: bool,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}

impl Default for StreamConfig {
    /// The endpoint and model id the old hardcoded harness used; labels left unlabeled.
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 18006,
            path: "/v1/chat/completions".to_string(),
            model: "atlas-lightning-omni".to_string(),
            engine: "unlabeled".to_string(),
            precision: None,
            context_length: None,
            max_tokens: 20,
            prompt: "Explain unified memory architecture in ten words.".to_string(),
            requests: 1,
            concurrency: 1,
            warmup: 1,
            request_usage: true,
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(20),
        }
    }
}

impl StreamConfig {
    pub fn endpoint(&self) -> String {
        format!("{}:{}{}", self.host, self.port, self.path)
    }

    pub fn request_body(&self) -> String {
        let mut body = json!({
            "model": self.model,
            "messages": [{"role": "user", "content": self.prompt}],
            "max_tokens": self.max_tokens,
            "stream": true,
        });
        if self.request_usage {
            body["stream_options"] = json!({"include_usage": true});
        }
        body.to_string()
    }

    pub fn request_bytes(&self) -> Vec<u8> {
        let body = self.request_body();
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        format!(
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.path,
            host,
            self.port,
            body.len(),
            body
        )
        .into_bytes()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// At least one choice carries non-empty `delta.content`, `delta.reasoning_content` or `text`.
    Content,
    /// Valid chunk without content: role-only, empty delta, finish_reason-only, usage-only.
    NonContent,
    Done,
    Error,
    Malformed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Classified {
    pub kind: EventKind,
    pub completion_tokens: Option<u64>,
    pub prompt_tokens: Option<u64>,
}

/// Classify one SSE event's `data` (and optional `event` name).
pub fn classify(event: Option<&str>, data: &str) -> Classified {
    let mut c = Classified {
        kind: EventKind::Malformed,
        completion_tokens: None,
        prompt_tokens: None,
    };
    if data.trim() == "[DONE]" {
        c.kind = EventKind::Done;
        return c;
    }
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(data) else {
        if event == Some("error") {
            c.kind = EventKind::Error;
        }
        return c;
    };
    if event == Some("error") || obj.get("error").is_some_and(|e| !e.is_null()) {
        c.kind = EventKind::Error;
        return c;
    }
    if let Some(usage) = obj.get("usage") {
        c.completion_tokens = usage.get("completion_tokens").and_then(Value::as_u64);
        c.prompt_tokens = usage.get("prompt_tokens").and_then(Value::as_u64);
    }
    let non_empty = |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    let has_content = obj
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices.iter().any(|ch| {
                let delta = ch.get("delta");
                non_empty(delta.and_then(|d| d.get("content")))
                    || non_empty(delta.and_then(|d| d.get("reasoning_content")))
                    || non_empty(ch.get("text"))
            })
        });
    c.kind = if has_content {
        EventKind::Content
    } else {
        EventKind::NonContent
    };
    c
}

/// How one request ended. Only `Complete` contributes observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Complete,
    ConnectError,
    WriteError,
    ReadError,
    ReadTimeout,
    MalformedStream,
    TruncatedHead,
    HttpStatus,
    StreamErrorEvent,
    TruncatedBody,
    ZeroEvents,
    MissingDone,
    NoContent,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Complete => "complete",
            Outcome::ConnectError => "connect_error",
            Outcome::WriteError => "write_error",
            Outcome::ReadError => "read_error",
            Outcome::ReadTimeout => "read_timeout",
            Outcome::MalformedStream => "malformed_stream",
            Outcome::TruncatedHead => "truncated_head",
            Outcome::HttpStatus => "http_status",
            Outcome::StreamErrorEvent => "stream_error_event",
            Outcome::TruncatedBody => "truncated_body",
            Outcome::ZeroEvents => "zero_events",
            Outcome::MissingDone => "missing_done",
            Outcome::NoContent => "no_content",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EventRecord {
    pub offset_ms: f64,
    pub read_index: u64,
    pub kind: EventKind,
}

/// Raw per-request observations. Offsets are milliseconds since `request_start`.
#[derive(Debug, Clone, Serialize)]
pub struct RequestSample {
    pub index: u32,
    pub outcome: Outcome,
    pub detail: Option<String>,
    pub connect_ms: Option<f64>,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub first_byte_ms: Option<f64>,
    pub head_ms: Option<f64>,
    pub first_content_ms: Option<f64>,
    pub last_content_ms: Option<f64>,
    pub completion_ms: Option<f64>,
    pub done_seen: bool,
    pub content_events: u64,
    pub non_content_events: u64,
    pub error_events: u64,
    pub malformed_events: u64,
    pub usage_completion_tokens: Option<u64>,
    pub usage_prompt_tokens: Option<u64>,
    pub reads: u64,
    pub bytes: u64,
    pub sse_comments: u64,
    pub sse_ignored_lines: u64,
    pub sse_invalid_utf8_lines: u64,
    pub sse_empty_dispatches: u64,
    pub error_body: Option<String>,
    pub events: Vec<EventRecord>,
}

impl RequestSample {
    fn empty(index: u32) -> Self {
        Self {
            index,
            outcome: Outcome::ReadError,
            detail: None,
            connect_ms: None,
            http_status: None,
            content_type: None,
            first_byte_ms: None,
            head_ms: None,
            first_content_ms: None,
            last_content_ms: None,
            completion_ms: None,
            done_seen: false,
            content_events: 0,
            non_content_events: 0,
            error_events: 0,
            malformed_events: 0,
            usage_completion_tokens: None,
            usage_prompt_tokens: None,
            reads: 0,
            bytes: 0,
            sse_comments: 0,
            sse_ignored_lines: 0,
            sse_invalid_utf8_lines: 0,
            sse_empty_dispatches: 0,
            error_body: None,
            events: Vec::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.outcome == Outcome::Complete
    }
}

/// Pure state machine for one streamed response: feed reads with their arrival instants.
pub struct StreamAccumulator {
    start: Instant,
    http: HttpResponseParser,
    sse: SseParser,
    body: Vec<u8>,
    events: Vec<SseEvent>,
    error_body: Vec<u8>,
    sample: RequestSample,
    failure: Option<(Outcome, String)>,
    finished: bool,
}

impl StreamAccumulator {
    pub fn new(index: u32, request_start: Instant) -> Self {
        Self {
            start: request_start,
            http: HttpResponseParser::new(),
            sse: SseParser::new(),
            body: Vec::new(),
            events: Vec::new(),
            error_body: Vec::new(),
            sample: RequestSample::empty(index),
            failure: None,
            finished: false,
        }
    }

    fn ms(&self, at: Instant) -> f64 {
        at.saturating_duration_since(self.start).as_secs_f64() * 1000.0
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Bytes of one read that returned at `at`.
    pub fn feed(&mut self, bytes: &[u8], at: Instant) {
        if self.finished {
            return;
        }
        let read_index = self.sample.reads;
        self.sample.reads += 1;
        self.sample.bytes += bytes.len() as u64;
        if self.sample.first_byte_ms.is_none() && !bytes.is_empty() {
            self.sample.first_byte_ms = Some(self.ms(at));
        }
        self.body.clear();
        if let Err(e) = self.http.feed(bytes, &mut self.body) {
            self.fail(Outcome::MalformedStream, e.to_string());
            return;
        }
        let Some(head) = self.http.head().cloned() else {
            return;
        };
        if self.sample.head_ms.is_none() {
            self.sample.head_ms = Some(self.ms(at));
            self.sample.http_status = Some(head.status);
            self.sample.content_type = head.content_type.clone();
        }
        if !head.is_success() {
            let room = MAX_ERROR_BODY_BYTES.saturating_sub(self.error_body.len());
            let take = room.min(self.body.len());
            self.error_body.extend_from_slice(&self.body[..take]);
            if self.http.body_complete() || room == 0 {
                self.finished = true;
            }
            return;
        }
        if let Err(e) = self.sse.feed(&self.body, at, read_index, &mut self.events) {
            self.fail(Outcome::MalformedStream, e.to_string());
            return;
        }
        let events = std::mem::take(&mut self.events);
        for ev in events {
            let c = classify(ev.event.as_deref(), &ev.data);
            let offset_ms = self.ms(ev.at);
            self.sample.events.push(EventRecord {
                offset_ms,
                read_index: ev.read_index,
                kind: c.kind,
            });
            if c.completion_tokens.is_some() {
                self.sample.usage_completion_tokens = c.completion_tokens;
            }
            if c.prompt_tokens.is_some() {
                self.sample.usage_prompt_tokens = c.prompt_tokens;
            }
            match c.kind {
                EventKind::Content => {
                    self.sample.content_events += 1;
                    if self.sample.first_content_ms.is_none() {
                        self.sample.first_content_ms = Some(offset_ms);
                    }
                    self.sample.last_content_ms = Some(offset_ms);
                }
                EventKind::NonContent => self.sample.non_content_events += 1,
                EventKind::Error => self.sample.error_events += 1,
                EventKind::Malformed => self.sample.malformed_events += 1,
                EventKind::Done => {
                    self.sample.done_seen = true;
                    self.sample.completion_ms = Some(offset_ms);
                    self.finished = true;
                    break;
                }
            }
        }
        if self.http.body_complete() {
            self.finished = true;
        }
    }

    /// The peer closed the connection (read returned 0).
    pub fn eof(&mut self) {
        self.finished = true;
    }

    /// A transport failure; the first one wins.
    pub fn fail(&mut self, outcome: Outcome, detail: String) {
        if self.failure.is_none() {
            self.failure = Some((outcome, detail));
        }
        self.finished = true;
    }

    pub fn finish(mut self) -> RequestSample {
        let s = &mut self.sample;
        s.sse_comments = self.sse.comments;
        s.sse_ignored_lines = self.sse.ignored_lines;
        s.sse_invalid_utf8_lines = self.sse.invalid_utf8_lines;
        s.sse_empty_dispatches = self.sse.empty_dispatches;
        if !self.error_body.is_empty() {
            let text = String::from_utf8_lossy(&self.error_body);
            s.error_body = Some(text.chars().take(ERROR_SNIPPET_CHARS).collect());
        }
        let total_events = s.events.len();
        let (outcome, detail) = if let Some((o, d)) = self.failure.take() {
            (o, Some(d))
        } else if s.http_status.is_none() {
            (
                Outcome::TruncatedHead,
                Some("stream ended before a complete HTTP response head".to_string()),
            )
        } else if !self.http.head().is_some_and(|h| h.is_success()) {
            (
                Outcome::HttpStatus,
                Some(format!("HTTP {}", s.http_status.unwrap_or(0))),
            )
        } else if s.error_events > 0 {
            (
                Outcome::StreamErrorEvent,
                Some(format!("{} error event(s) in stream", s.error_events)),
            )
        } else if s.done_seen {
            if s.content_events == 0 {
                (
                    Outcome::NoContent,
                    Some("data: [DONE] without any content event".to_string()),
                )
            } else {
                (Outcome::Complete, None)
            }
        } else if !self.http.complete_at_eof() || self.sse.has_pending() {
            (
                Outcome::TruncatedBody,
                Some("body ended before its framing or the last SSE event completed".to_string()),
            )
        } else if total_events == 0 {
            (Outcome::ZeroEvents, Some("no SSE events".to_string()))
        } else {
            (
                Outcome::MissingDone,
                Some("stream ended without data: [DONE]".to_string()),
            )
        };
        s.outcome = outcome;
        s.detail = detail;
        if outcome != Outcome::Complete {
            s.completion_ms = None;
        }
        self.sample
    }
}

/// One request over a fresh TCP connection (`Connection: close`).
pub fn measure_one(cfg: &StreamConfig, index: u32) -> RequestSample {
    let connect_start = Instant::now();
    let addr = match (cfg.host.as_str(), cfg.port)
        .to_socket_addrs()
        .map_err(|e| e.to_string())
        .and_then(|mut a| a.next().ok_or_else(|| "no address".to_string()))
    {
        Ok(a) => a,
        Err(e) => return failed(index, Outcome::ConnectError, format!("resolve: {}", e)),
    };
    let mut stream = match TcpStream::connect_timeout(&addr, cfg.connect_timeout) {
        Ok(s) => s,
        Err(e) => return failed(index, Outcome::ConnectError, format!("{}: {}", addr, e)),
    };
    let connect_ms = connect_start.elapsed().as_secs_f64() * 1000.0;
    if let Err(e) = stream
        .set_read_timeout(Some(cfg.read_timeout))
        .and_then(|_| stream.set_write_timeout(Some(cfg.read_timeout)))
        .and_then(|_| stream.set_nodelay(true))
    {
        return failed(index, Outcome::ConnectError, e.to_string());
    }
    let req = cfg.request_bytes();
    let request_start = Instant::now();
    let mut acc = StreamAccumulator::new(index, request_start);
    if let Err(e) = stream.write_all(&req) {
        acc.fail(Outcome::WriteError, e.to_string());
    }
    let mut buf = vec![0u8; 16 * 1024];
    while !acc.is_finished() {
        match stream.read(&mut buf) {
            Ok(0) => acc.eof(),
            Ok(n) => {
                let at = Instant::now();
                acc.feed(&buf[..n], at);
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => acc.fail(
                Outcome::ReadTimeout,
                format!("no bytes for {:?}: {}", cfg.read_timeout, e),
            ),
            Err(e) => acc.fail(Outcome::ReadError, e.to_string()),
        }
    }
    let mut sample = acc.finish();
    sample.connect_ms = Some(connect_ms);
    sample
}

fn failed(index: u32, outcome: Outcome, detail: String) -> RequestSample {
    let mut s = RequestSample::empty(index);
    s.outcome = outcome;
    s.detail = Some(detail);
    s
}

pub struct StreamRun {
    pub warmup: Vec<RequestSample>,
    pub samples: Vec<RequestSample>,
    pub wall: Duration,
}

/// Warmup requests run serially and are discarded; then `requests` measured requests run
/// on `concurrency` threads.
pub fn run_stream_benchmark(cfg: &StreamConfig) -> StreamRun {
    let warmup = (0..cfg.warmup).map(|i| measure_one(cfg, i)).collect();
    let next = AtomicU32::new(0);
    let results = Mutex::new(Vec::with_capacity(cfg.requests as usize));
    let workers = cfg.concurrency.clamp(1, cfg.requests.max(1));
    let t0 = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= cfg.requests {
                    break;
                }
                let s = measure_one(cfg, i);
                results.lock().unwrap_or_else(|p| p.into_inner()).push(s);
            });
        }
    });
    let wall = t0.elapsed();
    let mut samples = results.into_inner().unwrap_or_else(|p| p.into_inner());
    samples.sort_by_key(|s| s.index);
    StreamRun {
        warmup,
        samples,
        wall,
    }
}

/// Percentiles from real observations only.
#[derive(Debug, Clone, PartialEq)]
pub struct Distribution {
    pub samples: usize,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
}

fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    sorted[rank.clamp(1, n) - 1]
}

/// Nearest-rank percentiles: p50 when N >= 1, p95 only when N >= [`P95_MIN_SAMPLES`].
pub fn distribution(values: &[f64]) -> Distribution {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    Distribution {
        samples: n,
        p50: (n >= 1).then(|| nearest_rank(&sorted, 50.0)),
        p95: (n >= P95_MIN_SAMPLES).then(|| nearest_rank(&sorted, 95.0)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenAccounting {
    BackendUsage,
    EventsNotTokens,
}

impl TokenAccounting {
    pub fn as_str(self) -> &'static str {
        match self {
            TokenAccounting::BackendUsage => "backend_usage",
            TokenAccounting::EventsNotTokens => "events_not_tokens",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamSummary {
    pub requests_attempted: usize,
    pub requests_ok: usize,
    pub failures: BTreeMap<&'static str, usize>,
    pub wall_ms: f64,
    pub ttft: Distribution,
    pub first_byte: Distribution,
    pub e2e: Distribution,
    pub inter_event: Distribution,
    pub content_events_total: u64,
    pub events_coalesced_in_read: u64,
    pub malformed_events_total: u64,
    pub events_per_sec: Option<f64>,
    pub token_accounting: TokenAccounting,
    pub usage_requests: usize,
    pub output_tokens_total: Option<u64>,
    pub output_tokens_per_sec: Option<f64>,
    pub tpot: Option<Distribution>,
}

pub fn summarize(samples: &[RequestSample], wall: Duration) -> StreamSummary {
    let ok: Vec<&RequestSample> = samples.iter().filter(|s| s.is_complete()).collect();
    let mut failures = BTreeMap::new();
    for s in samples.iter().filter(|s| !s.is_complete()) {
        *failures.entry(s.outcome.as_str()).or_insert(0) += 1;
    }
    let mut gaps = Vec::new();
    let mut coalesced = 0;
    for s in &ok {
        let mut prev: Option<&EventRecord> = None;
        for e in s.events.iter().filter(|e| e.kind == EventKind::Content) {
            if let Some(p) = prev {
                gaps.push(e.offset_ms - p.offset_ms);
                if p.read_index == e.read_index {
                    coalesced += 1;
                }
            }
            prev = Some(e);
        }
    }
    let wall_s = wall.as_secs_f64();
    let content_events_total: u64 = ok.iter().map(|s| s.content_events).sum();
    let usage_requests = ok
        .iter()
        .filter(|s| s.usage_completion_tokens.is_some())
        .count();
    let token_accounting = if !ok.is_empty() && usage_requests == ok.len() {
        TokenAccounting::BackendUsage
    } else {
        TokenAccounting::EventsNotTokens
    };
    let (output_tokens_total, output_tokens_per_sec, tpot) =
        if token_accounting == TokenAccounting::BackendUsage {
            let total: u64 = ok.iter().filter_map(|s| s.usage_completion_tokens).sum();
            let tpot: Vec<f64> = ok
                .iter()
                .filter_map(|s| {
                    let tokens = s.usage_completion_tokens?;
                    let window = s.last_content_ms? - s.first_content_ms?;
                    (tokens >= 2 && window > 0.0).then(|| window / (tokens - 1) as f64)
                })
                .collect();
            (
                Some(total),
                (wall_s > 0.0).then(|| total as f64 / wall_s),
                Some(distribution(&tpot)),
            )
        } else {
            (None, None, None)
        };
    let pick = |f: fn(&RequestSample) -> Option<f64>| -> Vec<f64> {
        ok.iter().filter_map(|s| f(s)).collect()
    };
    StreamSummary {
        requests_attempted: samples.len(),
        requests_ok: ok.len(),
        failures,
        wall_ms: wall_s * 1000.0,
        ttft: distribution(&pick(|s| s.first_content_ms)),
        first_byte: distribution(&pick(|s| s.first_byte_ms)),
        e2e: distribution(&pick(|s| s.completion_ms)),
        inter_event: distribution(&gaps),
        content_events_total,
        events_coalesced_in_read: coalesced,
        malformed_events_total: ok.iter().map(|s| s.malformed_events).sum(),
        events_per_sec: (!ok.is_empty() && wall_s > 0.0)
            .then(|| content_events_total as f64 / wall_s),
        token_accounting,
        usage_requests,
        output_tokens_total,
        output_tokens_per_sec,
        tpot,
    }
}

#[derive(Debug, Clone)]
pub struct SourceIdentity {
    pub commit: String,
    pub dirty: bool,
}

#[derive(Debug, Clone)]
pub struct BuildIdentity {
    pub profile: String,
    pub rustc: String,
}

#[derive(Debug, Clone)]
pub struct HardwareIdentity {
    pub host: String,
    pub cpu: String,
    pub gpu: String,
}

/// Commit and dirty flag of the source tree this binary was built from, read with git at
/// run time. Refuses (Err) rather than inventing a commit.
pub fn source_identity() -> Result<SourceIdentity, String> {
    let dir = env!("CARGO_MANIFEST_DIR");
    let head = Command::new("git")
        .args(["-C", dir, "rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("git rev-parse failed: {}", e))?;
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if !head.status.success()
        || commit.len() != 40
        || !commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(format!("cannot read a 40-hex commit from {}", dir));
    }
    let status = Command::new("git")
        .args(["-C", dir, "status", "--porcelain", "--untracked-files=no"])
        .output()
        .map_err(|e| format!("git status failed: {}", e))?;
    if !status.status.success() {
        return Err(format!("git status failed in {}", dir));
    }
    Ok(SourceIdentity {
        commit,
        dirty: !status.stdout.is_empty(),
    })
}

pub fn build_identity() -> BuildIdentity {
    let rustc = Command::new("rustc")
        .arg("-V")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    BuildIdentity {
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .to_string(),
        rustc,
    }
}

/// Host name, CPU model and GPU model from /proc only (no GPU tool is run).
pub fn hardware_identity() -> HardwareIdentity {
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname")
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let mut gpus = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/proc/driver/nvidia/gpus") {
        for entry in dir.flatten() {
            if let Ok(info) = std::fs::read_to_string(entry.path().join("information")) {
                if let Some((_, v)) = info
                    .lines()
                    .find(|l| l.starts_with("Model:"))
                    .and_then(|l| l.split_once(':'))
                {
                    let model = v.trim();
                    gpus.push(if model.is_empty() || model == "Unknown" {
                        "NVIDIA GPU (driver reports model Unknown)".to_string()
                    } else {
                        model.to_string()
                    });
                }
            }
        }
    }
    gpus.sort();
    HardwareIdentity {
        host,
        cpu,
        gpu: if gpus.is_empty() {
            "none_detected".to_string()
        } else {
            gpus.join(", ")
        },
    }
}

fn put_dist(m: &mut Map<String, Value>, name: &str, d: &Distribution) {
    if let Some(v) = d.p50 {
        m.insert(format!("{}_p50", name), json!(v));
    }
    if let Some(v) = d.p95 {
        m.insert(format!("{}_p95", name), json!(v));
    }
    m.insert(format!("{}_samples", name), json!(d.samples));
}

/// The `aien-bench-record/1` JSON for one run.
pub fn build_record(
    cfg: &StreamConfig,
    summary: &StreamSummary,
    source: &SourceIdentity,
    build: &BuildIdentity,
    hw: &HardwareIdentity,
    evidence: &[String],
) -> Value {
    let mut m = Map::new();
    m.insert(
        "requests_attempted".into(),
        json!(summary.requests_attempted),
    );
    m.insert("requests_ok".into(), json!(summary.requests_ok));
    m.insert(
        "requests_failed".into(),
        json!(summary.requests_attempted - summary.requests_ok),
    );
    m.insert("wall_ms".into(), json!(summary.wall_ms));
    put_dist(&mut m, "ttft_ms", &summary.ttft);
    put_dist(&mut m, "first_byte_ms", &summary.first_byte);
    put_dist(&mut m, "e2e_ms", &summary.e2e);
    put_dist(&mut m, "inter_event_ms", &summary.inter_event);
    m.insert(
        "content_events_total".into(),
        json!(summary.content_events_total),
    );
    m.insert(
        "events_coalesced_in_read".into(),
        json!(summary.events_coalesced_in_read),
    );
    if let Some(v) = summary.events_per_sec {
        m.insert("events_per_sec".into(), json!(v));
    }
    if let Some(v) = summary.output_tokens_total {
        m.insert("output_tokens_total".into(), json!(v));
    }
    if let Some(v) = summary.output_tokens_per_sec {
        m.insert("output_tokens_per_sec".into(), json!(v));
    }
    if let Some(d) = &summary.tpot {
        put_dist(&mut m, "tpot_ms", d);
    }

    let mut defs = Map::new();
    let mut def = |k: &str, v: String| {
        defs.insert(k.to_string(), json!(v));
    };
    def("request_start", "Instant taken immediately before the HTTP request bytes are written to an already-connected TCP socket; connect time is excluded and kept per request in the samples file as connect_ms".into());
    def("arrival", "an event's arrival is the Instant taken immediately after the socket read that delivered its final byte returned; events completed by one read share that arrival".into());
    def(
        "first_byte_ms",
        "request_start to arrival of the first response byte".into(),
    );
    def("ttft_ms", "request_start to arrival of the first content event: an SSE event whose JSON has a non-empty choices[].delta.content, choices[].delta.reasoning_content or choices[].text. Role-only, empty-delta, finish-only and usage-only events are not content".into());
    def("e2e_ms", "request_start to arrival of data: [DONE]".into());
    def("inter_event_ms", "gap between arrivals of consecutive content events of one request, pooled over successful requests; SSE events, not model tokens".into());
    def("events_coalesced_in_read", "content events that arrived in the same socket read as the previous content event of their request; their inter-event gap is 0 by construction".into());
    def(
        "events_per_sec",
        "content events of successful requests divided by wall_ms; events, not tokens".into(),
    );
    def(
        "wall_ms",
        "first measured request dispatch to last measured request finish; warmup excluded".into(),
    );
    def("success", "a request succeeds when it returns HTTP 2xx, has at least one content event, no error event, and ends with data: [DONE]; read errors and read timeouts are failures, never end of stream".into());
    def("percentiles", format!("nearest-rank over the N successful observations of that metric; p50 when N >= 1, p95 only when N >= {}; N is recorded as <metric>_samples", P95_MIN_SAMPLES));
    def("warmup", format!("{} warmup request(s) with the same prompt and max_tokens ran serially first and were discarded", cfg.warmup));
    if summary.token_accounting == TokenAccounting::BackendUsage {
        def("output_tokens_per_sec", "sum of backend-reported usage.completion_tokens over successful requests divided by wall_ms".into());
        def("tpot_ms", "per request: (last content event arrival - first content event arrival) / (usage.completion_tokens - 1); requests with fewer than 2 tokens are excluded".into());
    }

    let mut limitations = vec![
        "Commit and dirty flag were read from the source tree with git at run time, not embedded at build time; rustc is the toolchain on PATH at run time.".to_string(),
        "Arrival times have socket-read granularity: bytes that arrive together in one read cannot be told apart.".to_string(),
        "Not reviewed or qualified against a pinned baseline; EXPERIMENTAL until it is.".to_string(),
    ];
    match summary.token_accounting {
        TokenAccounting::EventsNotTokens if summary.requests_ok == 0 => limitations.push(
            "events_not_tokens: no request succeeded, so no event or token metrics exist.".to_string(),
        ),
        TokenAccounting::EventsNotTokens => limitations.push(format!(
            "events_not_tokens: the backend reported usage.completion_tokens for {} of {} successful requests, so no token counts, tokens per second or per-token latency are reported; event metrics count SSE events, which may carry zero, one or several tokens.",
            summary.usage_requests, summary.requests_ok
        )),
        TokenAccounting::BackendUsage => limitations.push(
            "Token counts are the backend's own usage.completion_tokens, not verified by this client.".to_string(),
        ),
    }
    if summary.events_coalesced_in_read > 0 {
        limitations.push(format!(
            "{} content event(s) arrived in the same read as the previous one; their 0 ms gaps are in inter_event_ms.",
            summary.events_coalesced_in_read
        ));
    }
    if summary.ttft.samples < P95_MIN_SAMPLES {
        limitations.push(format!(
            "Only {} successful request(s): p95 values are omitted (need {}).",
            summary.ttft.samples, P95_MIN_SAMPLES
        ));
    }
    if !summary.failures.is_empty() {
        let kinds: Vec<String> = summary
            .failures
            .iter()
            .map(|(k, n)| format!("{} {}", n, k))
            .collect();
        limitations.push(format!(
            "{} of {} measured request(s) failed ({}); metrics use successful requests only.",
            summary.requests_attempted - summary.requests_ok,
            summary.requests_attempted,
            kinds.join(", ")
        ));
    }
    if summary.malformed_events_total > 0 {
        limitations.push(format!(
            "{} SSE event(s) in successful requests were not valid JSON and were ignored.",
            summary.malformed_events_total
        ));
    }
    if cfg.precision.is_none() {
        limitations.push("Precision was not labeled by the operator.".to_string());
    }
    if cfg.engine == "unlabeled" {
        limitations.push("Engine was not labeled by the operator.".to_string());
    }
    if !cfg.request_usage {
        limitations.push(
            "stream_options.include_usage was not requested, so backends that only send usage on request did not send it.".to_string(),
        );
    }

    let mut backend = json!({
        "engine": cfg.engine,
        "model": cfg.model,
        "precision": cfg.precision.clone().unwrap_or_else(|| "unlabeled".to_string()),
        "endpoint": cfg.endpoint(),
    });
    if let Some(ctx) = cfg.context_length {
        backend["context_length"] = json!(ctx);
    }

    json!({
        "schema": RECORD_SCHEMA,
        "status": if summary.requests_ok == 0 { "FAIL" } else { "EXPERIMENTAL" },
        "source": {"repo": REPO, "commit": source.commit, "dirty": source.dirty},
        "build": {"profile": build.profile, "rustc": build.rustc},
        "workload": {
            "name": "llm_streaming",
            "prompt_bytes": cfg.prompt.len(),
            "max_tokens": cfg.max_tokens,
            "requests": cfg.requests,
            "concurrency": cfg.concurrency,
        },
        "hardware": {"host": hw.host, "cpu": hw.cpu, "gpu": hw.gpu},
        "backend": backend,
        "methodology": {
            "clock": CLOCK,
            "definitions": Value::Object(defs),
            "token_accounting": summary.token_accounting.as_str(),
            "samples": summary.ttft.samples,
        },
        "metrics": Value::Object(m),
        "evidence": evidence,
        "limitations": limitations,
    })
}

/// Write `<stem>.samples.json` (raw per-request observations) and `<stem>.json` (the
/// record) into `out_dir`. Evidence paths are relative to the record's directory.
pub fn write_run(
    out_dir: &Path,
    cfg: &StreamConfig,
    run: &StreamRun,
    summary: &StreamSummary,
    source: &SourceIdentity,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {}", out_dir.display(), e))?;
    let stem = format!(
        "llm_streaming-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
    );
    let samples_name = format!("{}.samples.json", stem);
    let record_name = format!("{}.json", stem);
    let samples = json!({
        "schema": SAMPLES_SCHEMA,
        "record": record_name,
        "clock": CLOCK,
        "offsets": "milliseconds since request_start of the same request",
        "warmup": run.warmup,
        "samples": run.samples,
    });
    let record = build_record(
        cfg,
        summary,
        source,
        &build_identity(),
        &hardware_identity(),
        std::slice::from_ref(&samples_name),
    );
    let write = |name: &str, v: &Value| {
        let path = out_dir.join(name);
        let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
        std::fs::write(&path, text + "\n").map_err(|e| format!("write {}: {}", path.display(), e))
    };
    write(&samples_name, &samples)?;
    write(&record_name, &record)?;
    Ok(out_dir.join(record_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::net::TcpListener;

    const HEAD_CHUNKED: &[u8] =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
    const HEAD_CLOSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";

    fn content(text: &str) -> String {
        format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            text
        )
    }

    fn chunk(payload: &str) -> Vec<u8> {
        let mut v = format!("{:x}\r\n", payload.len()).into_bytes();
        v.extend_from_slice(payload.as_bytes());
        v.extend_from_slice(b"\r\n");
        v
    }

    /// Feed `reads` as separate reads, read i arriving at base + 10*(i+1) ms, then EOF.
    fn run_reads(reads: &[Vec<u8>]) -> RequestSample {
        let base = Instant::now();
        let mut acc = StreamAccumulator::new(0, base);
        for (i, r) in reads.iter().enumerate() {
            acc.feed(r, base + Duration::from_millis(10 * (i as u64 + 1)));
            if acc.is_finished() {
                return acc.finish();
            }
        }
        acc.eof();
        acc.finish()
    }

    fn cat(parts: &[&[u8]]) -> Vec<u8> {
        parts.concat()
    }

    #[test]
    fn frame_split_across_reads_uses_completing_read_time() {
        let ev = content("hi");
        let (a, b) = ev.as_bytes().split_at(10);
        let s = run_reads(&[
            HEAD_CLOSE.to_vec(),
            a.to_vec(),
            b.to_vec(),
            b"data: [DONE]\n\n".to_vec(),
        ]);
        assert_eq!(s.outcome, Outcome::Complete);
        assert_eq!(s.head_ms, Some(10.0));
        assert_eq!(
            s.first_content_ms,
            Some(30.0),
            "stamped at the read with the final byte"
        );
        assert_eq!(s.completion_ms, Some(40.0));
        assert_eq!(s.content_events, 1);
    }

    #[test]
    fn multiple_events_in_one_read_are_coalesced_and_counted() {
        let body = format!("{}{}{}", content("a"), content("b"), content("c"));
        let s = run_reads(&[
            cat(&[HEAD_CLOSE, body.as_bytes()]),
            content("d").into_bytes(),
            b"data: [DONE]\n\n".to_vec(),
        ]);
        assert_eq!(s.outcome, Outcome::Complete);
        assert_eq!(s.content_events, 4);
        let sum = summarize(std::slice::from_ref(&s), Duration::from_millis(100));
        assert_eq!(sum.events_coalesced_in_read, 2);
        assert_eq!(sum.inter_event.samples, 3);
        assert_eq!(sum.inter_event.p50, Some(0.0));
        assert_eq!(sum.inter_event.p95, None);
        assert_eq!(sum.token_accounting, TokenAccounting::EventsNotTokens);
        assert!((sum.events_per_sec.unwrap() - 40.0).abs() < 1e-9);
    }

    #[test]
    fn malformed_stream_is_a_failure_and_bad_json_is_counted() {
        let bad_chunk = run_reads(&[cat(&[HEAD_CHUNKED, b"zz\r\nxx"])]);
        assert_eq!(bad_chunk.outcome, Outcome::MalformedStream);
        assert_eq!(bad_chunk.first_content_ms, None);

        let bad_json = run_reads(&[
            cat(&[HEAD_CLOSE, b"data: {not json\n\n"]),
            content("ok").into_bytes(),
            b"data: [DONE]\n\n".to_vec(),
        ]);
        assert_eq!(bad_json.outcome, Outcome::Complete);
        assert_eq!(bad_json.malformed_events, 1);
        assert_eq!(bad_json.first_content_ms, Some(20.0));
    }

    #[test]
    fn zero_events_and_done_only_are_failures() {
        let zero = run_reads(&[cat(&[HEAD_CLOSE, b": ping\n\n"])]);
        assert_eq!(zero.outcome, Outcome::ZeroEvents);
        assert_eq!(zero.sse_comments, 1);

        let empty = run_reads(&[]);
        assert_eq!(empty.outcome, Outcome::TruncatedHead);

        let done_only = run_reads(&[cat(&[HEAD_CLOSE, b"data: [DONE]\n\n"])]);
        assert_eq!(done_only.outcome, Outcome::NoContent);
        let sum = summarize(&[zero, done_only], Duration::from_millis(5));
        assert_eq!(sum.requests_ok, 0);
        assert_eq!(sum.ttft.samples, 0);
        assert_eq!(sum.ttft.p50, None);
        assert_eq!(sum.events_per_sec, None);
    }

    #[test]
    fn http_500_body_is_kept_and_never_measured() {
        let s = run_reads(&[
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 25\r\n\r\n{\"error\":".to_vec(),
            b"\"model crashed\"}".to_vec(),
        ]);
        assert_eq!(s.outcome, Outcome::HttpStatus);
        assert_eq!(s.http_status, Some(500));
        assert_eq!(
            s.error_body.as_deref(),
            Some("{\"error\":\"model crashed\"}")
        );
        assert_eq!(s.first_content_ms, None);
        assert_eq!(s.events.len(), 0);
    }

    #[test]
    fn error_event_in_a_200_stream_fails_the_request() {
        let s = run_reads(&[cat(&[
            HEAD_CLOSE,
            content("a").as_bytes(),
            b"data: {\"error\":{\"message\":\"oom\"}}\n\ndata: [DONE]\n\n",
        ])]);
        assert_eq!(s.outcome, Outcome::StreamErrorEvent);
        assert_eq!(s.error_events, 1);
        assert_eq!(s.completion_ms, None);
    }

    #[test]
    fn non_token_events_do_not_start_ttft() {
        let role = "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n";
        let empty = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\"}}]}\n\n";
        let finish =
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";
        let s = run_reads(&[
            cat(&[HEAD_CHUNKED, &chunk(role)]),
            chunk(empty),
            chunk(&content("x")),
            chunk(finish),
            cat(&[&chunk("data: [DONE]\n\n"), b"0\r\n\r\n"]),
        ]);
        assert_eq!(s.outcome, Outcome::Complete);
        assert_eq!(s.non_content_events, 3);
        assert_eq!(s.content_events, 1);
        assert_eq!(s.first_byte_ms, Some(10.0));
        assert_eq!(s.first_content_ms, Some(30.0));
    }

    #[test]
    fn truncated_responses_are_failures_not_eof() {
        // Chunked body cut mid-event.
        let s = run_reads(&[cat(&[
            HEAD_CHUNKED,
            &chunk(&content("a")),
            b"20\r\ndata: {\"cho",
        ])]);
        assert_eq!(s.outcome, Outcome::TruncatedBody);
        assert_eq!(s.content_events, 1);
        // Read-until-close body that closes cleanly but never sends [DONE].
        let s = run_reads(&[cat(&[HEAD_CLOSE, content("a").as_bytes()])]);
        assert_eq!(s.outcome, Outcome::MissingDone);
        // Head cut short.
        let s = run_reads(&[b"HTTP/1.1 200 OK\r\nContent-Ty".to_vec()]);
        assert_eq!(s.outcome, Outcome::TruncatedHead);
        // A read error after content is a failure.
        let base = Instant::now();
        let mut acc = StreamAccumulator::new(0, base);
        acc.feed(&cat(&[HEAD_CLOSE, content("a").as_bytes()]), base);
        acc.fail(Outcome::ReadError, "connection reset".into());
        assert_eq!(acc.finish().outcome, Outcome::ReadError);
    }

    #[test]
    fn usage_present_vs_absent_accounting() {
        let usage =
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":9}}\n\n";
        let with_usage = run_reads(&[
            cat(&[HEAD_CLOSE, content("a").as_bytes()]),
            content("bcd").into_bytes(),
            content("e").into_bytes(),
            cat(&[usage.as_bytes(), b"data: [DONE]\n\n"]),
        ]);
        assert_eq!(with_usage.usage_completion_tokens, Some(9));
        assert_eq!(with_usage.usage_prompt_tokens, Some(7));
        let sum = summarize(std::slice::from_ref(&with_usage), Duration::from_secs(1));
        assert_eq!(sum.token_accounting, TokenAccounting::BackendUsage);
        assert_eq!(sum.output_tokens_total, Some(9));
        assert_eq!(sum.output_tokens_per_sec, Some(9.0));
        // first content at 10 ms, last at 30 ms, 9 tokens: 20 / 8 ms per token.
        assert_eq!(sum.tpot.as_ref().unwrap().p50, Some(2.5));

        let without = run_reads(&[
            cat(&[HEAD_CLOSE, content("a").as_bytes()]),
            cat(&[content("b").as_bytes(), b"data: [DONE]\n\n"]),
        ]);
        let mixed = summarize(&[with_usage, without], Duration::from_secs(1));
        assert_eq!(mixed.token_accounting, TokenAccounting::EventsNotTokens);
        assert_eq!(mixed.usage_requests, 1);
        assert_eq!(mixed.output_tokens_per_sec, None);
        assert!(mixed.tpot.is_none());

        let rec = build_record(
            &StreamConfig::default(),
            &mixed,
            &SourceIdentity {
                commit: "0".repeat(40),
                dirty: false,
            },
            &BuildIdentity {
                profile: "debug".into(),
                rustc: "rustc test".into(),
            },
            &HardwareIdentity {
                host: "h".into(),
                cpu: "c".into(),
                gpu: "none_detected".into(),
            },
            &["x.samples.json".to_string()],
        );
        assert_eq!(rec["methodology"]["token_accounting"], "events_not_tokens");
        let metrics = rec["metrics"].as_object().unwrap();
        assert!(
            metrics
                .keys()
                .all(|k| !k.contains("token") && !k.contains("tpot") && !k.contains("itl")),
            "no token metric under events_not_tokens: {:?}",
            metrics.keys()
        );
        assert!(metrics.contains_key("events_per_sec"));
    }

    #[test]
    fn percentile_rule_needs_enough_samples() {
        let one = distribution(&[42.0]);
        assert_eq!((one.samples, one.p50, one.p95), (1, Some(42.0), None));
        let nineteen: Vec<f64> = (1..=19).map(f64::from).collect();
        assert_eq!(distribution(&nineteen).p95, None);
        let twenty: Vec<f64> = (1..=20).map(f64::from).collect();
        let d = distribution(&twenty);
        assert_eq!((d.samples, d.p50, d.p95), (20, Some(10.0), Some(19.0)));
        assert_eq!(distribution(&[]).p50, None);
    }

    #[test]
    fn single_sample_record_has_p50_only() {
        let s = run_reads(&[
            cat(&[HEAD_CLOSE, content("a").as_bytes()]),
            cat(&[content("b").as_bytes(), b"data: [DONE]\n\n"]),
        ]);
        let sum = summarize(&[s], Duration::from_millis(50));
        let rec = build_record(
            &StreamConfig::default(),
            &sum,
            &SourceIdentity {
                commit: "a".repeat(40),
                dirty: true,
            },
            &build_identity(),
            &hardware_identity(),
            &["x.samples.json".to_string()],
        );
        assert_eq!(rec["schema"], RECORD_SCHEMA);
        assert_eq!(rec["status"], "EXPERIMENTAL");
        assert_eq!(rec["methodology"]["samples"], 1);
        assert_eq!(rec["metrics"]["ttft_ms_p50"], 10.0);
        assert_eq!(rec["metrics"]["ttft_ms_samples"], 1);
        assert!(rec["metrics"].get("ttft_ms_p95").is_none());
        assert_eq!(rec["backend"]["model"], "atlas-lightning-omni");
        assert_eq!(rec["backend"]["precision"], "unlabeled");
        assert_eq!(
            rec["backend"]["endpoint"],
            "127.0.0.1:18006/v1/chat/completions"
        );
        for key in [
            "source",
            "build",
            "workload",
            "hardware",
            "evidence",
            "limitations",
        ] {
            assert!(rec.get(key).is_some(), "missing {}", key);
        }
    }

    #[test]
    fn request_body_escapes_prompt_and_asks_for_usage() {
        let cfg = StreamConfig {
            prompt: "say \"hi\"\nnow".into(),
            ..StreamConfig::default()
        };
        let v: Value = serde_json::from_str(&cfg.request_body()).unwrap();
        assert_eq!(v["messages"][0]["content"], "say \"hi\"\nnow");
        assert_eq!(v["stream"], true);
        assert_eq!(v["stream_options"]["include_usage"], true);
    }

    /// Loopback fake server: answers each connection with `script` (pieces written with a
    /// pause between them), then closes.
    fn fake_server(
        connections: usize,
        script: Vec<Vec<u8>>,
        pause: Duration,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for _ in 0..connections {
                let (mut sock, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(sock.try_clone().unwrap());
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                bodies.push(String::from_utf8(body).unwrap());
                for piece in &script {
                    if sock.write_all(piece).is_err() {
                        break;
                    }
                    let _ = sock.flush();
                    std::thread::sleep(pause);
                }
            }
            bodies
        });
        (port, handle)
    }

    #[test]
    fn end_to_end_over_loopback_with_concurrency() {
        let usage = "data: {\"choices\":[],\"usage\":{\"completion_tokens\":3}}\n\n";
        let script = vec![
            HEAD_CHUNKED.to_vec(),
            chunk(&content("a")),
            chunk(&content("b")),
            chunk(&format!("{}{}data: [DONE]\n\n", content("c"), usage)),
            b"0\r\n\r\n".to_vec(),
        ];
        let (port, server) = fake_server(3, script, Duration::from_millis(5));
        let cfg = StreamConfig {
            port,
            requests: 2,
            concurrency: 2,
            warmup: 1,
            model: "fake-model".into(),
            engine: "fake-engine".into(),
            ..StreamConfig::default()
        };
        let run = run_stream_benchmark(&cfg);
        let bodies = server.join().unwrap();
        assert_eq!(bodies.len(), 3);
        assert!(bodies
            .iter()
            .all(|b| b.contains("\"model\":\"fake-model\"")));
        assert_eq!(run.warmup.len(), 1);
        assert_eq!(run.samples.len(), 2);
        for s in &run.samples {
            assert_eq!(s.outcome, Outcome::Complete, "{:?}", s.detail);
            assert_eq!(s.content_events, 3);
            assert!(s.first_content_ms.unwrap() <= s.completion_ms.unwrap());
        }
        let sum = summarize(&run.samples, run.wall);
        assert_eq!(sum.requests_ok, 2);
        assert_eq!(sum.token_accounting, TokenAccounting::BackendUsage);
        assert_eq!(sum.ttft.samples, 2);
        assert!(sum.ttft.p95.is_none());

        let dir = std::env::temp_dir().join(format!("aien-bench-p0-{}", std::process::id()));
        let path = write_run(
            &dir,
            &cfg,
            &run,
            &sum,
            &SourceIdentity {
                commit: "b".repeat(40),
                dirty: false,
            },
        )
        .unwrap();
        let rec: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let evidence = rec["evidence"][0].as_str().unwrap();
        assert!(dir.join(evidence).is_file());
        assert_eq!(rec["backend"]["engine"], "fake-engine");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn loopback_http_500_and_stall_and_refused() {
        let (port, server) = fake_server(
            1,
            vec![b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\n\r\nbusy".to_vec()],
            Duration::from_millis(1),
        );
        let cfg = StreamConfig {
            port,
            ..StreamConfig::default()
        };
        let s = measure_one(&cfg, 0);
        server.join().unwrap();
        assert_eq!(s.outcome, Outcome::HttpStatus);
        assert_eq!(s.error_body.as_deref(), Some("busy"));

        // Head and one event, then silence longer than the read timeout.
        let (port, server) = fake_server(
            1,
            vec![cat(&[HEAD_CLOSE, content("a").as_bytes()])],
            Duration::from_millis(600),
        );
        let cfg = StreamConfig {
            port,
            read_timeout: Duration::from_millis(150),
            ..StreamConfig::default()
        };
        let s = measure_one(&cfg, 0);
        server.join().unwrap();
        assert_eq!(s.outcome, Outcome::ReadTimeout, "{:?}", s.detail);
        assert_eq!(s.content_events, 1);

        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let cfg = StreamConfig {
            port,
            ..StreamConfig::default()
        };
        assert_eq!(measure_one(&cfg, 0).outcome, Outcome::ConnectError);
    }
}
