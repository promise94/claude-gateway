//! 观测上游响应流：统计字节数、分块数、耗时，判断流是否正常结束。
//!
//! 存在的理由：访问日志只记录到「响应头已返回」，无法区分「流正常收完」
//! 和「上游中途断开」。客户端报 `Server error mid-response` 时，
//! 只有这里能说明是谁先断的：
//!
//! - `上游流中断`（WARN，带 error）→ 上游连接出错
//! - `上游流提前结束`（WARN，无结束标记）→ 上游正常关闭但没有发完
//! - `客户端提前断开`（DEBUG）→ 下游主动断开，与上游无关

use bytes::Bytes;
use futures_core::Stream;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use crate::telemetry::{self, StreamRecord};

/// SSE 流正常结束的标记，任一出现即认为完整
const END_MARKERS: &[&[u8]] = &[b"message_stop", b"[DONE]", b"response.completed"];

/// 跨块检测标记时保留的尾部长度，够覆盖最长的标记
const CARRY_BYTES: usize = 64;

pub struct ObservedStream<S> {
    inner: S,
    /// 与访问日志同源的请求 ID，便于把流异常对应到具体请求
    rid: String,
    path: String,
    is_sse: bool,
    started: Instant,
    bytes: u64,
    chunks: u64,
    carry: Vec<u8>,
    saw_end: bool,
    finished: bool,
}

impl<S> ObservedStream<S> {
    pub fn new(inner: S, path: impl Into<String>, is_sse: bool, rid: impl Into<String>) -> Self {
        Self {
            inner,
            rid: rid.into(),
            path: path.into(),
            is_sse,
            started: Instant::now(),
            bytes: 0,
            chunks: 0,
            carry: Vec::new(),
            saw_end: false,
            finished: false,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// 累积数据并检测结束标记。标记可能被切在两个 chunk 之间，
    /// 所以保留一小段尾部拼接后再判断。
    fn scan(&mut self, chunk: &[u8]) {
        if self.saw_end {
            return;
        }
        self.carry.extend_from_slice(chunk);
        self.saw_end = has_end_marker(&self.carry);

        if self.carry.len() > CARRY_BYTES {
            let cut = self.carry.len() - CARRY_BYTES;
            self.carry.drain(..cut);
        }
    }

    fn log_end(&self) {
        if !self.is_sse {
            tracing::debug!(
                rid = %self.rid,
                path = %self.path,
                bytes = self.bytes,
                elapsed_ms = self.elapsed_ms(),
                "响应完成"
            );
            return;
        }

        if self.saw_end {
            tracing::debug!(
                rid = %self.rid,
                path = %self.path,
                bytes = self.bytes,
                chunks = self.chunks,
                elapsed_ms = self.elapsed_ms(),
                "流式响应完成"
            );
            self.record("complete", None);
        } else {
            tracing::warn!(
                rid = %self.rid,
                path = %self.path,
                bytes = self.bytes,
                chunks = self.chunks,
                elapsed_ms = self.elapsed_ms(),
                "上游流提前结束（未见结束标记）"
            );
            self.record("truncated", None);
        }
    }

    /// 只记 SSE：普通 JSON 响应不是流，混进控制台的「流事件」表会稀释信号
    fn record(&self, outcome: &'static str, detail: Option<String>) {
        if !self.is_sse {
            return;
        }
        telemetry::submit_stream(StreamRecord {
            at: telemetry::now_local(),
            rid: self.rid.clone(),
            path: self.path.clone(),
            outcome,
            bytes: self.bytes,
            chunks: self.chunks,
            elapsed_ms: self.elapsed_ms(),
            detail,
        });
    }
}

impl<S, E> Stream for ObservedStream<S>
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::fmt::Display,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        match Pin::new(&mut this.inner).poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Ok(chunk))) => {
                this.chunks += 1;
                this.bytes += chunk.len() as u64;
                if this.is_sse {
                    this.scan(&chunk);
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                tracing::warn!(
                    rid = %this.rid,
                    path = %this.path,
                    bytes = this.bytes,
                    chunks = this.chunks,
                    elapsed_ms = this.elapsed_ms(),
                    error = %error,
                    "上游流中断（响应未完成）"
                );
                this.record("error", Some(error.to_string()));
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.finished = true;
                this.log_end();
                Poll::Ready(None)
            }
        }
    }
}

impl<S> Drop for ObservedStream<S> {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // 客户端断开时 hyper 会直接丢掉 body，这里不会走到 None
        tracing::debug!(
            rid = %self.rid,
            path = %self.path,
            bytes = self.bytes,
            chunks = self.chunks,
            elapsed_ms = self.elapsed_ms(),
            "客户端提前断开，上游流未读完"
        );
        self.record("client_closed", None);
    }
}

/// 判断缓冲区里是否出现任一结束标记
fn has_end_marker(buf: &[u8]) -> bool {
    END_MARKERS
        .iter()
        .any(|marker| buf.windows(marker.len()).any(|window| window == *marker))
}

#[cfg(test)]
mod tests {
    use super::has_end_marker;

    #[test]
    fn detects_marker_in_single_chunk() {
        assert!(has_end_marker(b"event: message_stop\ndata: {}\n\n"));
        assert!(has_end_marker(b"data: [DONE]\n\n"));
        assert!(has_end_marker(b"\"type\":\"response.completed\""));
    }

    #[test]
    fn detects_marker_split_across_chunks() {
        let mut carry = Vec::new();
        carry.extend_from_slice(b"event: mess");
        assert!(!has_end_marker(&carry));
        carry.extend_from_slice(b"age_stop\n\n");
        assert!(has_end_marker(&carry));
    }

    #[test]
    fn ignores_partial_marker() {
        assert!(!has_end_marker(b"event: message_delta\n"));
        assert!(!has_end_marker(b"data: {\"type\":\"message_start\"}\n\n"));
    }
}
