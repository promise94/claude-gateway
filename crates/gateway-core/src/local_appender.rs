//! 按本地日期切分的日志文件写入器
//!
//! 为什么不用 `tracing_appender::rolling`：它的 `Rotation::DAILY` 按 UTC 日期命名，
//! 跨天点是 UTC 00:00。北京时间下表现为 0:00–8:00 的日志写进前一天的文件，
//! 和日志行里的本地时间戳对不上，排查时容易把两天的事混在一起。
//!
//! 这里每次写入前比对本地日期，变了就切换文件，并清理超出保留数量的旧文件。
//! 只删自己命名规则内的文件，目录里的其他内容一概不碰。

use chrono::{Local, NaiveDate};
use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

/// 日期来源。生产用本地时区，测试注入固定日期，才能覆盖跨天逻辑。
pub type DateSource = Arc<dyn Fn() -> NaiveDate + Send + Sync>;

pub fn local_today() -> NaiveDate {
    Local::now().date_naive()
}

struct State {
    date: NaiveDate,
    file: BufWriter<File>,
}

pub struct LocalDailyAppender {
    dir: PathBuf,
    prefix: String,
    suffix: String,
    retain: usize,
    today: DateSource,
    state: Option<State>,
}

impl LocalDailyAppender {
    pub fn new(
        dir: impl Into<PathBuf>,
        prefix: &str,
        suffix: &str,
        retain: usize,
    ) -> io::Result<Self> {
        Self::with_clock(dir, prefix, suffix, retain, Arc::new(local_today))
    }

    pub fn with_clock(
        dir: impl Into<PathBuf>,
        prefix: &str,
        suffix: &str,
        retain: usize,
        today: DateSource,
    ) -> io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            prefix: prefix.to_string(),
            suffix: suffix.to_string(),
            retain: retain.max(1),
            today,
            state: None,
        })
    }

    fn file_name(&self, date: NaiveDate) -> String {
        format!(
            "{}.{}.{}",
            self.prefix,
            date.format("%Y-%m-%d"),
            self.suffix
        )
    }

    fn open(&mut self, date: NaiveDate) -> io::Result<()> {
        let path = self.dir.join(self.file_name(date));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        self.state = Some(State {
            date,
            file: BufWriter::new(file),
        });
        self.cleanup(date);
        Ok(())
    }

    /// 按文件名（即日期）排序，只保留最近 `retain` 个
    fn cleanup(&self, current: NaiveDate) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| self.matches(name))
            .collect();
        names.sort();

        let excess = names.len().saturating_sub(self.retain);
        for name in names.into_iter().take(excess) {
            // 正在写的文件永不删除，即使保留数量配成了 0
            if name == self.file_name(current) {
                continue;
            }
            let _ = std::fs::remove_file(self.dir.join(&name));
        }
    }

    fn matches(&self, name: &str) -> bool {
        let Some(rest) = name.strip_prefix(&format!("{}.", self.prefix)) else {
            return false;
        };
        let Some(date) = rest.strip_suffix(&format!(".{}", self.suffix)) else {
            return false;
        };
        NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
    }

    fn ensure_current(&mut self) -> io::Result<()> {
        let today = (self.today)();
        match &self.state {
            Some(state) if state.date == today => Ok(()),
            _ => self.open(today),
        }
    }
}

impl Write for LocalDailyAppender {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.ensure_current()?;
        match &mut self.state {
            Some(state) => state.file.write(buf),
            None => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.state {
            Some(state) => state.file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cg-appender-{tag}-{}-{seq}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    fn date(raw: &str) -> NaiveDate {
        NaiveDate::parse_from_str(raw, "%Y-%m-%d").expect("合法日期")
    }

    #[test]
    fn writes_to_file_named_after_local_date() {
        let dir = temp_dir("local");
        let mut appender = LocalDailyAppender::with_clock(
            &dir,
            "gateway",
            "log",
            7,
            Arc::new(|| date("2026-11-06")),
        )
        .expect("创建 appender");

        appender.write_all(b"line-a\n").expect("写入");
        appender.flush().expect("flush");

        let path = dir.join("gateway.2026-11-06.log");
        assert_eq!(
            std::fs::read_to_string(&path).expect("读回"),
            "line-a\n",
            "文件名日期必须等于写入时的本地日期"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 这条是本次 bug 的回归用例：跨天必须按本地日期切，不能等 UTC 零点
    #[test]
    fn rolls_over_at_local_midnight() {
        let dir = temp_dir("roll");
        let today = Arc::new(Mutex::new(date("2026-11-05")));
        let clock = {
            let today = today.clone();
            Arc::new(move || *today.lock().expect("日期锁被投毒"))
        };
        let mut appender = LocalDailyAppender::with_clock(&dir, "gateway", "log", 7, clock)
            .expect("创建 appender");

        appender.write_all(b"before\n").expect("写入前一天");
        *today.lock().expect("日期锁被投毒") = date("2026-11-06");
        appender.write_all(b"after\n").expect("写入当天");
        appender.flush().expect("flush");

        assert_eq!(
            std::fs::read_to_string(dir.join("gateway.2026-11-05.log")).expect("读前一天"),
            "before\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("gateway.2026-11-06.log")).expect("读当天"),
            "after\n"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn appends_when_restarted_same_day() {
        let dir = temp_dir("append");
        for content in ["first\n", "second\n"] {
            let mut appender = LocalDailyAppender::with_clock(
                &dir,
                "gateway",
                "log",
                7,
                Arc::new(|| date("2026-11-06")),
            )
            .expect("创建 appender");
            appender.write_all(content.as_bytes()).expect("写入");
            appender.flush().expect("flush");
        }

        assert_eq!(
            std::fs::read_to_string(dir.join("gateway.2026-11-06.log")).expect("读回"),
            "first\nsecond\n",
            "同一天重启应追加而不是覆盖"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cleanup_keeps_newest_and_ignores_other_files() {
        let dir = temp_dir("retain");
        for day in ["2026-11-01", "2026-11-02", "2026-11-03"] {
            std::fs::write(dir.join(format!("gateway.{day}.log")), "old\n").expect("铺旧文件");
        }
        std::fs::write(dir.join("notes.txt"), "keep").expect("放无关文件");

        let mut appender = LocalDailyAppender::with_clock(
            &dir,
            "gateway",
            "log",
            3,
            Arc::new(|| date("2026-11-04")),
        )
        .expect("创建 appender");
        appender.write_all(b"new\n").expect("写入");
        appender.flush().expect("flush");

        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .expect("读目录")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "gateway.2026-11-02.log",
                "gateway.2026-11-03.log",
                "gateway.2026-11-04.log",
                "notes.txt"
            ],
            "只保留最近 3 个日志文件，无关文件不动"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
