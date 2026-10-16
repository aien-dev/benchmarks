//! Incremental framing for a streamed HTTP/1.1 response carrying Server-Sent Events.
//!
//! Pure code: no sockets, no clock reads. The caller feeds the bytes of each socket read
//! together with the `Instant` at which that read returned; every event completed by those
//! bytes is stamped with that instant. That is the earliest moment this process could have
//! observed the event's final byte, so it is the event's arrival time. Several events
//! completed by the same read share one arrival time and the same `read_index`; callers
//! report that coalescing instead of hiding it.
//!
//! Layers, each bounded:
//! 1. [`HttpResponseParser`]: status line and headers (at most [`MAX_HEAD_BYTES`]), then the
//!    body decoded from `Transfer-Encoding: chunked`, `Content-Length`, or read-until-close.
//! 2. [`SseParser`]: SSE lines (LF or CRLF endings, at most [`MAX_LINE_BYTES`] per line),
//!    comments, `data`/`event`/`id` fields, blank-line dispatch. A lone CR is not treated as
//!    a line ending (it stays part of the line).

use std::fmt;
use std::time::Instant;

/// Largest accepted status line plus headers.
pub const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Largest accepted single SSE line (and chunk-size or trailer line).
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
const MAX_CHUNK_LINE_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    HeadTooLarge,
    BadStatusLine(String),
    BadContentLength(String),
    BadChunkSize(String),
    BadChunkTerminator,
    ChunkLineTooLong,
    SseLineTooLong,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::HeadTooLarge => write!(f, "response head exceeds {} bytes", MAX_HEAD_BYTES),
            FrameError::BadStatusLine(l) => write!(f, "bad HTTP status line: {:?}", l),
            FrameError::BadContentLength(v) => write!(f, "bad Content-Length: {:?}", v),
            FrameError::BadChunkSize(l) => write!(f, "bad chunk size line: {:?}", l),
            FrameError::BadChunkTerminator => write!(f, "chunk data not followed by CRLF"),
            FrameError::ChunkLineTooLong => write!(f, "chunk size or trailer line too long"),
            FrameError::SseLineTooLong => write!(f, "SSE line exceeds {} bytes", MAX_LINE_BYTES),
        }
    }
}

/// How the end of the response body is recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFraming {
    Chunked,
    Length(u64),
    UntilClose,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseHead {
    pub status: u16,
    pub content_type: Option<String>,
    pub framing: BodyFraming,
}

impl ResponseHead {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChunkState {
    Size,
    Data(u64),
    DataEnd,
    Trailers,
    Done,
}

/// Incremental HTTP/1.1 response parser: head first, then decoded body bytes.
#[derive(Debug)]
pub struct HttpResponseParser {
    head_buf: Vec<u8>,
    head: Option<ResponseHead>,
    chunk: ChunkState,
    line: Vec<u8>,
    remaining: u64,
    body_complete: bool,
}

impl Default for HttpResponseParser {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpResponseParser {
    pub fn new() -> Self {
        Self {
            head_buf: Vec::new(),
            head: None,
            chunk: ChunkState::Size,
            line: Vec::new(),
            remaining: 0,
            body_complete: false,
        }
    }

    pub fn head(&self) -> Option<&ResponseHead> {
        self.head.as_ref()
    }

    /// True once the body framing says the body has ended (chunked terminator seen or
    /// Content-Length satisfied). Read-until-close bodies only end at EOF.
    pub fn body_complete(&self) -> bool {
        self.body_complete
    }

    /// True if, at EOF, the body is complete under its framing.
    pub fn complete_at_eof(&self) -> bool {
        match self.head.as_ref().map(|h| h.framing) {
            None => false,
            Some(BodyFraming::UntilClose) => true,
            Some(_) => self.body_complete,
        }
    }

    /// Feed raw bytes from one read; decoded body bytes are appended to `body_out`.
    pub fn feed(&mut self, mut bytes: &[u8], body_out: &mut Vec<u8>) -> Result<(), FrameError> {
        if self.head.is_none() {
            let before = self.head_buf.len();
            self.head_buf.extend_from_slice(bytes);
            match find_head_end(&self.head_buf) {
                Some(end) => {
                    let head = parse_head(&self.head_buf[..end])?;
                    if let BodyFraming::Length(n) = head.framing {
                        self.remaining = n;
                        self.body_complete = n == 0;
                    }
                    self.head = Some(head);
                    let consumed_from_this = end - before;
                    bytes = &bytes[consumed_from_this..];
                    self.head_buf = Vec::new();
                }
                None => {
                    if self.head_buf.len() > MAX_HEAD_BYTES {
                        return Err(FrameError::HeadTooLarge);
                    }
                    return Ok(());
                }
            }
        }
        let framing = self.head.as_ref().map(|h| h.framing);
        match framing {
            Some(BodyFraming::UntilClose) => body_out.extend_from_slice(bytes),
            Some(BodyFraming::Length(_)) => {
                let take = (self.remaining.min(bytes.len() as u64)) as usize;
                body_out.extend_from_slice(&bytes[..take]);
                self.remaining -= take as u64;
                if self.remaining == 0 {
                    self.body_complete = true;
                }
            }
            Some(BodyFraming::Chunked) => self.feed_chunked(bytes, body_out)?,
            None => {}
        }
        Ok(())
    }

    fn feed_chunked(&mut self, bytes: &[u8], body_out: &mut Vec<u8>) -> Result<(), FrameError> {
        let mut i = 0;
        while i < bytes.len() {
            match self.chunk {
                ChunkState::Done => return Ok(()),
                ChunkState::Data(n) => {
                    let take = (n.min((bytes.len() - i) as u64)) as usize;
                    body_out.extend_from_slice(&bytes[i..i + take]);
                    i += take;
                    let left = n - take as u64;
                    self.chunk = if left == 0 {
                        ChunkState::DataEnd
                    } else {
                        ChunkState::Data(left)
                    };
                }
                ChunkState::Size | ChunkState::DataEnd | ChunkState::Trailers => {
                    let b = bytes[i];
                    i += 1;
                    if b != b'\n' {
                        if self.line.len() >= MAX_CHUNK_LINE_BYTES {
                            return Err(FrameError::ChunkLineTooLong);
                        }
                        self.line.push(b);
                        continue;
                    }
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    let line = std::mem::take(&mut self.line);
                    match self.chunk {
                        ChunkState::Size => {
                            let text = String::from_utf8_lossy(&line).to_string();
                            let hex = text.split(';').next().unwrap_or("").trim();
                            let n = u64::from_str_radix(hex, 16)
                                .map_err(|_| FrameError::BadChunkSize(text.clone()))?;
                            self.chunk = if n == 0 {
                                ChunkState::Trailers
                            } else {
                                ChunkState::Data(n)
                            };
                        }
                        ChunkState::DataEnd => {
                            if !line.is_empty() {
                                return Err(FrameError::BadChunkTerminator);
                            }
                            self.chunk = ChunkState::Size;
                        }
                        _ => {
                            if line.is_empty() {
                                self.chunk = ChunkState::Done;
                                self.body_complete = true;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2);
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn parse_head(raw: &[u8]) -> Result<ResponseHead, FrameError> {
    let text = String::from_utf8_lossy(raw);
    let mut lines = text.lines();
    let status_line = lines.next().unwrap_or("").trim_end_matches('\r');
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    let status = parts
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|_| version.starts_with("HTTP/1."))
        .ok_or_else(|| FrameError::BadStatusLine(status_line.to_string()))?;
    let mut content_type = None;
    let mut chunked = false;
    let mut length = None;
    for line in lines {
        let line = line.trim_end_matches('\r');
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-type" => content_type = Some(value.to_string()),
            "transfer-encoding" => {
                chunked = value
                    .split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("chunked"))
            }
            "content-length" => {
                length = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| FrameError::BadContentLength(value.to_string()))?,
                )
            }
            _ => {}
        }
    }
    let framing = if chunked {
        BodyFraming::Chunked
    } else if let Some(n) = length {
        BodyFraming::Length(n)
    } else {
        BodyFraming::UntilClose
    };
    Ok(ResponseHead {
        status,
        content_type,
        framing,
    })
}

/// One dispatched SSE event.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    /// When the read that delivered this event's final byte (the blank line) returned.
    pub at: Instant,
    /// Index of that read (0-based, counted by the caller).
    pub read_index: u64,
}

/// Incremental SSE line parser. Counters record what was skipped, so nothing is dropped
/// silently.
#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    data: String,
    has_data: bool,
    event: Option<String>,
    /// `:` comment lines (keep-alives).
    pub comments: u64,
    /// Lines naming a field other than data/event/id/retry (ignored per the SSE rules).
    pub ignored_lines: u64,
    /// Lines that are not valid UTF-8 (skipped).
    pub invalid_utf8_lines: u64,
    /// Blank lines that ended an event without any data (nothing dispatched).
    pub empty_dispatches: u64,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed decoded body bytes delivered by read `read_index`, which returned at `at`.
    pub fn feed(
        &mut self,
        bytes: &[u8],
        at: Instant,
        read_index: u64,
        out: &mut Vec<SseEvent>,
    ) -> Result<(), FrameError> {
        for &b in bytes {
            if b != b'\n' {
                if self.line.len() >= MAX_LINE_BYTES {
                    return Err(FrameError::SseLineTooLong);
                }
                self.line.push(b);
                continue;
            }
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            let line = std::mem::take(&mut self.line);
            self.process_line(&line, at, read_index, out);
        }
        Ok(())
    }

    /// True if bytes of an unfinished line or event are buffered (a truncated stream).
    pub fn has_pending(&self) -> bool {
        !self.line.is_empty() || self.has_data
    }

    fn process_line(&mut self, line: &[u8], at: Instant, read_index: u64, out: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if self.has_data {
                out.push(SseEvent {
                    event: self.event.take(),
                    data: std::mem::take(&mut self.data),
                    at,
                    read_index,
                });
                self.has_data = false;
            } else {
                self.event = None;
                self.empty_dispatches += 1;
            }
            return;
        }
        let Ok(text) = std::str::from_utf8(line) else {
            self.invalid_utf8_lines += 1;
            return;
        };
        if text.starts_with(':') {
            self.comments += 1;
            return;
        }
        let (field, value) = match text.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (text, ""),
        };
        match field {
            "data" => {
                if self.has_data {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                self.has_data = true;
            }
            "event" => self.event = Some(value.to_string()),
            "id" | "retry" => {}
            _ => self.ignored_lines += 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn t(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn event_split_across_reads_is_stamped_with_the_completing_read() {
        let base = Instant::now();
        let mut p = SseParser::new();
        let mut out = Vec::new();
        p.feed(b"data: {\"a\"", t(base, 1), 0, &mut out).unwrap();
        assert!(out.is_empty());
        assert!(p.has_pending());
        p.feed(b":1}\n", t(base, 5), 1, &mut out).unwrap();
        assert!(out.is_empty());
        p.feed(b"\n", t(base, 9), 2, &mut out).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, "{\"a\":1}");
        assert_eq!(out[0].at, t(base, 9));
        assert_eq!(out[0].read_index, 2);
        assert!(!p.has_pending());
    }

    #[test]
    fn several_events_in_one_read_share_that_read() {
        let base = Instant::now();
        let mut p = SseParser::new();
        let mut out = Vec::new();
        p.feed(
            b"data: one\r\n\r\n: keep-alive\r\ndata: two\n\nevent: x\ndata: a\ndata: b\n\n",
            t(base, 3),
            7,
            &mut out,
        )
        .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].data, "one");
        assert_eq!(out[1].data, "two");
        assert_eq!(out[2].data, "a\nb");
        assert_eq!(out[2].event.as_deref(), Some("x"));
        assert!(out.iter().all(|e| e.read_index == 7 && e.at == t(base, 3)));
        assert_eq!(p.comments, 1);
    }

    #[test]
    fn malformed_lines_are_counted_not_dispatched() {
        let base = Instant::now();
        let mut p = SseParser::new();
        let mut out = Vec::new();
        p.feed(b"garbage line\nfoo: bar\n\xff\xfe\n\n", base, 0, &mut out)
            .unwrap();
        assert!(out.is_empty());
        assert_eq!(p.ignored_lines, 2);
        assert_eq!(p.invalid_utf8_lines, 1);
        assert_eq!(p.empty_dispatches, 1);
    }

    #[test]
    fn overlong_line_is_an_error() {
        let mut p = SseParser::new();
        let mut out = Vec::new();
        let big = vec![b'x'; MAX_LINE_BYTES + 1];
        assert_eq!(
            p.feed(&big, Instant::now(), 0, &mut out),
            Err(FrameError::SseLineTooLong)
        );
    }

    #[test]
    fn chunked_body_split_anywhere_decodes_identically() {
        let raw: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n6\r\ndata: \r\n7;ext=1\r\nhello\n\n\r\n0\r\nX-Trailer: y\r\n\r\n";
        for split in 0..raw.len() {
            let mut h = HttpResponseParser::new();
            let mut body = Vec::new();
            h.feed(&raw[..split], &mut body).unwrap();
            h.feed(&raw[split..], &mut body).unwrap();
            assert_eq!(body, b"data: hello\n\n", "split at {}", split);
            assert!(h.body_complete(), "split at {}", split);
            let head = h.head().unwrap();
            assert_eq!(head.status, 200);
            assert_eq!(head.framing, BodyFraming::Chunked);
            assert_eq!(head.content_type.as_deref(), Some("text/event-stream"));
        }
    }

    #[test]
    fn content_length_body_and_truncation() {
        let mut h = HttpResponseParser::new();
        let mut body = Vec::new();
        h.feed(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 10\r\n\r\nboom",
            &mut body,
        )
        .unwrap();
        assert_eq!(h.head().unwrap().status, 500);
        assert!(!h.head().unwrap().is_success());
        assert!(!h.complete_at_eof());
        h.feed(b"!!!!!!extra", &mut body).unwrap();
        assert_eq!(body, b"boom!!!!!!");
        assert!(h.complete_at_eof());
    }

    #[test]
    fn bad_framing_is_reported() {
        let mut body = Vec::new();
        assert!(matches!(
            HttpResponseParser::new().feed(b"SSH-2.0-x\r\n\r\n", &mut body),
            Err(FrameError::BadStatusLine(_))
        ));
        assert!(matches!(
            HttpResponseParser::new().feed(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
                &mut body
            ),
            Err(FrameError::BadChunkSize(_))
        ));
        assert_eq!(
            HttpResponseParser::new().feed(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabXX\r\n",
                &mut body
            ),
            Err(FrameError::BadChunkTerminator)
        );
        let mut h = HttpResponseParser::new();
        assert_eq!(
            h.feed(&vec![b'a'; MAX_HEAD_BYTES + 1], &mut body),
            Err(FrameError::HeadTooLarge)
        );
    }

    #[test]
    fn eof_rules_per_framing() {
        let mut body = Vec::new();
        let mut h = HttpResponseParser::new();
        assert!(!h.complete_at_eof(), "no head yet");
        h.feed(b"HTTP/1.1 200 OK\r\n\r\ndata: x\n\n", &mut body)
            .unwrap();
        assert!(h.complete_at_eof(), "read-until-close ends at EOF");
        let mut c = HttpResponseParser::new();
        c.feed(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n",
            &mut body,
        )
        .unwrap();
        assert!(
            !c.complete_at_eof(),
            "chunked without terminator is truncated"
        );
    }
}
