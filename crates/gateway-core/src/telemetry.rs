//! 运行时可观测数据：请求、流事件、日志尾部
//!
//! 面板（`/_panel`）读这里的数据。全部是内存环形缓冲，进程重启即清空——
//! 长期记录仍然看落盘日志，这里只服务「刚才发生了什么」。
//!
//! 刻意不存请求体和响应体，理由同 `logging.rs`。

use serde::Serialize;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime};
use tracing_subscriber::fmt::MakeWriter;

const REQUEST_CAPACITY: usize = 200;
const STREAM_CAPACITY: usize = 200;
const LOG_CAPACITY: usize = 1000;

/// 全局实例。单进程服务，用一个 OnceLock 省掉层层传递。
static GLOBAL: OnceLock<Arc<Telemetry>> = OnceLock::new();

pub fn init_global() -> Arc<Telemetry> {
    GLOBAL.get_or_init(|| Arc::new(Telemetry::new())).clone()
}

pub fn global() -> Option<Arc<Telemetry>> {
    GLOBAL.get().cloned()
}

#[derive(Clone, Serialize)]
pub struct RequestRecord {
    pub at: String,
    /// 请求 ID，与访问日志里的 `rid=` 和流事件里的 rid 对应
    pub rid: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub elapsed_ms: u64,
    pub upstream_ms: u64,
    pub rewritten: String,
}

#[derive(Clone, Serialize)]
pub struct StreamRecord {
    pub at: String,
    /// 同一次请求的 ID，用来和访问日志、控制台请求行对照
    pub rid: String,
    pub path: String,
    /// complete | truncated | error | client_closed
    pub outcome: &'static str,
    pub bytes: u64,
    pub chunks: u64,
    pub elapsed_ms: u64,
    pub detail: Option<String>,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    errors: AtomicU64,
    streams: AtomicU64,
    stream_truncated: AtomicU64,
    stream_errors: AtomicU64,
    client_closed: AtomicU64,
}

#[derive(Serialize)]
pub struct CounterSnapshot {
    pub requests: u64,
    pub errors: u64,
    pub streams: u64,
    pub stream_truncated: u64,
    pub stream_errors: u64,
    pub client_closed: u64,
}

pub struct Telemetry {
    started: Instant,
    started_wall: SystemTime,
    counters: Counters,
    requests: Mutex<VecDeque<RequestRecord>>,
    streams: Mutex<VecDeque<StreamRecord>>,
    logs: Mutex<VecDeque<String>>,
    /// 由 main 在日志初始化后填入，面板上展示
    log_dir: Mutex<Option<String>>,
    config_source: Mutex<Option<String>>,
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            started_wall: SystemTime::now(),
            counters: Counters::default(),
            requests: Mutex::new(VecDeque::with_capacity(REQUEST_CAPACITY)),
            streams: Mutex::new(VecDeque::with_capacity(STREAM_CAPACITY)),
            logs: Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)),
            log_dir: Mutex::new(None),
            config_source: Mutex::new(None),
        }
    }

    pub fn set_paths(&self, log_dir: Option<String>, config_source: Option<String>) {
        *self.log_dir.lock().unwrap() = log_dir;
        *self.config_source.lock().unwrap() = config_source;
    }

    pub fn uptime_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn started_wall(&self) -> SystemTime {
        self.started_wall
    }

    pub fn log_dir(&self) -> Option<String> {
        self.log_dir.lock().unwrap().clone()
    }

    pub fn config_source(&self) -> Option<String> {
        self.config_source.lock().unwrap().clone()
    }

    pub fn record_request(&self, record: RequestRecord) {
        self.counters.requests.fetch_add(1, Ordering::Relaxed);
        if record.status >= 400 {
            self.counters.errors.fetch_add(1, Ordering::Relaxed);
        }
        let mut queue = self.requests.lock().unwrap();
        if queue.len() == REQUEST_CAPACITY {
            queue.pop_front();
        }
        queue.push_back(record);
    }

    pub fn record_stream(&self, record: StreamRecord) {
        self.counters.streams.fetch_add(1, Ordering::Relaxed);
        match record.outcome {
            "truncated" => self
                .counters
                .stream_truncated
                .fetch_add(1, Ordering::Relaxed),
            "error" => self.counters.stream_errors.fetch_add(1, Ordering::Relaxed),
            "client_closed" => self.counters.client_closed.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
        let mut queue = self.streams.lock().unwrap();
        if queue.len() == STREAM_CAPACITY {
            queue.pop_front();
        }
        queue.push_back(record);
    }

    pub fn push_log(&self, line: String) {
        let mut queue = self.logs.lock().unwrap();
        if queue.len() == LOG_CAPACITY {
            queue.pop_front();
        }
        queue.push_back(line);
    }

    pub fn counters(&self) -> CounterSnapshot {
        CounterSnapshot {
            requests: self.counters.requests.load(Ordering::Relaxed),
            errors: self.counters.errors.load(Ordering::Relaxed),
            streams: self.counters.streams.load(Ordering::Relaxed),
            stream_truncated: self.counters.stream_truncated.load(Ordering::Relaxed),
            stream_errors: self.counters.stream_errors.load(Ordering::Relaxed),
            client_closed: self.counters.client_closed.load(Ordering::Relaxed),
        }
    }

    /// 最近的请求，按时间倒序
    pub fn recent_requests(&self, limit: usize) -> Vec<RequestRecord> {
        let queue = self.requests.lock().unwrap();
        queue.iter().rev().take(limit).cloned().collect()
    }

    pub fn recent_streams(&self, limit: usize) -> Vec<StreamRecord> {
        let queue = self.streams.lock().unwrap();
        queue.iter().rev().take(limit).cloned().collect()
    }

    /// 日志尾部，按时间正序（最新在最后，方便面板滚动）
    pub fn recent_logs(&self, limit: usize, level: Option<&str>) -> Vec<String> {
        let queue = self.logs.lock().unwrap();
        // 先按级别筛，再取尾部 N 条：先取尾部再筛会把符合条件的行筛没
        let matched: Vec<&String> = queue
            .iter()
            .filter(|line| match level {
                Some(level) => contains_level(line, level),
                None => true,
            })
            .collect();
        let skip = matched.len().saturating_sub(limit);
        matched[skip..].iter().map(|line| (*line).clone()).collect()
    }

    /// 当前保留的日志行数（未按级别筛）
    pub fn log_len(&self) -> usize {
        self.logs.lock().unwrap().len()
    }
}

/// 本地时间，格式 `YYYY-MM-DD HH:mm:ss.SSS`
pub fn now_local() -> String {
    chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string()
}

/// 记录一条请求。全局实例未初始化时静默丢弃（单测、库里调用都会走到这里）。
pub fn submit_request(record: RequestRecord) {
    if let Some(telemetry) = global() {
        telemetry.record_request(record);
    }
}

/// 记录一条流事件。同上。
pub fn submit_stream(record: StreamRecord) {
    if let Some(telemetry) = global() {
        telemetry.record_stream(record);
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

fn contains_level(line: &str, level: &str) -> bool {
    let needle = format!(" {} ", level.to_uppercase());
    line.contains(&needle)
}

/// `tracing` 的终端 writer：原样写 stdout，同时把每一行剥掉 ANSI 后塞进环形缓冲。
///
/// 做成一个 writer 而不是「终端层 + 面板层」两个 tracing Layer，是因为
/// 在 `registry()` 上叠加三个不同的 `fmt` 层会让类型推断解不开。一个 writer
/// 只格式化一次，也省掉一次多余的格式化开销。
#[derive(Clone)]
pub struct TeeWriter {
    telemetry: Arc<Telemetry>,
}

impl TeeWriter {
    pub fn new(telemetry: Arc<Telemetry>) -> Self {
        Self { telemetry }
    }
}

impl<'a> MakeWriter<'a> for TeeWriter {
    type Writer = TeeLine;

    fn make_writer(&'a self) -> Self::Writer {
        TeeLine {
            telemetry: self.telemetry.clone(),
            buffer: Vec::new(),
        }
    }
}

pub struct TeeLine {
    telemetry: Arc<Telemetry>,
    buffer: Vec<u8>,
}

impl TeeLine {
    /// 把缓冲里已完整的行搬进环形缓冲
    fn drain_lines(&mut self) {
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]).to_string();
            let text = strip_ansi(&text);
            if !text.trim().is_empty() {
                self.telemetry.push_log(text);
            }
        }
    }
}

impl Write for TeeLine {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        // 终端这一路保持原样：交互时仍然要看到颜色
        let mut stdout = io::stdout();
        stdout.write_all(data)?;

        self.buffer.extend_from_slice(data);
        self.drain_lines();
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }
}

/// 去掉 ANSI 转义序列。面板里显示 `\x1b[2m` 之类的垃圾比没颜色更糟。
fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        // CSI：ESC [ ... 终止字节 (0x40..=0x7e)。其余转义序列一并吞到终止字节为止。
        let mut saw_bracket = false;
        for next in chars.by_ref() {
            if next == '[' && !saw_bracket {
                saw_bracket = true;
                continue;
            }
            if ('\u{40}'..='\u{7e}').contains(&next) {
                break;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(index: usize) -> RequestRecord {
        RequestRecord {
            at: format!("00:00:{index:02}"),
            rid: format!("{index:06x}"),
            method: "POST".into(),
            path: "/v1/messages".into(),
            status: 200,
            elapsed_ms: 1,
            upstream_ms: 1,
            rewritten: "-".into(),
        }
    }

    #[test]
    fn requests_are_capped_and_newest_first() {
        let telemetry = Telemetry::new();
        for index in 0..=REQUEST_CAPACITY {
            telemetry.record_request(record(index));
        }
        assert_eq!(telemetry.counters().requests, REQUEST_CAPACITY as u64 + 1);
        let recent = telemetry.recent_requests(5);
        assert_eq!(recent[0].at, format!("00:00:{REQUEST_CAPACITY:02}"));
        assert_eq!(recent.len(), 5);
    }

    #[test]
    fn errors_are_counted_separately() {
        let telemetry = Telemetry::new();
        let mut failed = record(0);
        failed.status = 502;
        telemetry.record_request(failed);
        telemetry.record_request(record(1));
        assert_eq!(telemetry.counters().errors, 1);
        assert_eq!(telemetry.counters().requests, 2);
    }

    #[test]
    fn stream_outcomes_are_counted() {
        let telemetry = Telemetry::new();
        for outcome in ["complete", "truncated", "error", "client_closed"] {
            telemetry.record_stream(StreamRecord {
                at: "00:00:00".into(),
                rid: format!("rid-{outcome}"),
                path: "/v1/messages".into(),
                outcome,
                bytes: 100,
                chunks: 2,
                elapsed_ms: 10,
                detail: None,
            });
        }
        let counters = telemetry.counters();
        assert_eq!(counters.streams, 4);
        assert_eq!(counters.stream_truncated, 1);
        assert_eq!(counters.stream_errors, 1);
        assert_eq!(counters.client_closed, 1);
    }

    #[test]
    fn logs_filter_by_level_and_keep_tail() {
        let telemetry = Telemetry::new();
        telemetry.push_log("2026-11-03 19:00:00.000  INFO 第一条".into());
        telemetry.push_log("2026-11-03 19:00:01.000  WARN 第二条".into());
        telemetry.push_log("2026-11-03 19:00:02.000  INFO 第三条".into());

        let all = telemetry.recent_logs(2, None);
        assert_eq!(all.len(), 2);
        assert!(all[1].contains("第三条"));

        let warns = telemetry.recent_logs(10, Some("warn"));
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("第二条"));
    }

    #[test]
    fn tee_line_splits_lines_and_strips_ansi() {
        let telemetry = Arc::new(Telemetry::new());
        let mut line = TeeLine {
            telemetry: telemetry.clone(),
            buffer: Vec::new(),
        };
        line.buffer
            .extend_from_slice("first\n\x1b[2msecond\x1b[0m\nhalf".as_bytes());
        line.drain_lines();
        assert_eq!(
            telemetry.recent_logs(10, None),
            vec!["first".to_string(), "second".to_string()]
        );
        // 不完整的行留在缓冲里，等下一次写入补齐
        assert_eq!(line.buffer, b"half".to_vec());
    }

    #[test]
    fn strip_ansi_removes_escape_sequences() {
        assert_eq!(strip_ansi("\u{1b}[2m普通\u{1b}[0m"), "普通");
        assert_eq!(strip_ansi("无转义"), "无转义");
        assert_eq!(strip_ansi("\u{1b}[1;31m红\u{1b}[m"), "红");
    }
}
