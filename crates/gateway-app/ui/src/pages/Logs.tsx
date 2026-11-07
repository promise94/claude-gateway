import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Button, Checkbox, Input, Select, Space, Typography } from "antd";

import {
  useExportLogs,
  useLogFile,
  useLogFiles,
  useLogLevel,
  useLogs,
  useOpenLogDir,
} from "../hooks/useGateway";
import { fmtBytes, levelOf } from "../lib/format";

const { Title, Text } = Typography;

const LEVELS = [
  { value: "all", label: "全部" },
  { value: "info", label: "INFO" },
  { value: "warn", label: "WARN" },
  { value: "error", label: "ERROR" },
  { value: "debug", label: "DEBUG" },
];

/** 与后端单页上限一致；一页 400 行在 460px 高的框里大约是十几屏 */
const PAGE_SIZE = 400;

type Source = "memory" | "file";
type Notice = { tone: "success" | "info" | "warning" | "error"; text: string } | null;

/** 优先用异步剪贴板；自定义协议下被拒时退回 execCommand */
async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    let copied = false;
    try {
      copied = document.execCommand("copy");
    } finally {
      document.body.removeChild(area);
    }
    return copied;
  }
}

export function Logs() {
  const [source, setSource] = useState<Source>("memory");
  const [level, setLevel] = useState("all");
  const [keyword, setKeyword] = useState("");
  const [paused, setPaused] = useState(false);
  const [follow, setFollow] = useState(true);
  const [fileName, setFileName] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  // 文件模式下让内存日志停轮询：看不见的数据不必每 2 秒取一次
  const memory = useLogs(level, paused || source !== "memory");
  const files = useLogFiles(source === "file");
  const file = useLogFile(source === "file" ? fileName : null, PAGE_SIZE);
  const levelState = useLogLevel();
  const exportLogs = useExportLogs();
  const openDir = useOpenLogDir();

  // 首次进入文件模式时默认选最新一天，省掉一次手动选择
  useEffect(() => {
    if (source !== "file" || fileName) return;
    const latest = files.data?.files[0]?.name;
    if (latest) setFileName(latest);
  }, [source, fileName, files.data]);

  const memoryLines = memory.data?.lines ?? [];

  // pages 顺序是「尾部 → 更早」，渲染要反过来才是时间正序
  const fileLines = useMemo(() => {
    const pages = file.data?.pages ?? [];
    return [...pages].reverse().flatMap((page) => page.lines);
  }, [file.data]);

  const rawLines = source === "memory" ? memoryLines : fileLines;

  const lines = useMemo(() => {
    const needle = keyword.trim().toLowerCase();
    return rawLines.filter((line) => {
      // 内存模式的级别筛选在服务端做过了，文件模式是整段读回来的，这里补上
      if (source === "file" && level !== "all" && levelOf(line) !== level.toUpperCase()) {
        return false;
      }
      return needle ? line.toLowerCase().includes(needle) : true;
    });
  }, [rawLines, keyword, level, source]);

  useEffect(() => {
    const box = boxRef.current;
    if (!box || !follow || source !== "memory") return;
    // 只在日志自身溢出时滚动，否则会牵连整个页面
    if (box.scrollHeight > box.clientHeight + 1) {
      box.scrollTop = box.scrollHeight;
    }
  }, [lines, follow, source]);

  const tail = file.data?.pages[0];
  const head = file.data?.pages[file.data.pages.length - 1];
  const range = tail && head ? { start: head.start, end: tail.end, total: tail.total } : null;

  const isLoading = source === "memory" ? memory.isLoading : files.isLoading || file.isLoading;
  const loadError = source === "memory" ? memory.error : (file.error ?? files.error);
  const canLoadEarlier = source === "file" && file.hasNextPage && !file.isFetchingNextPage;

  async function handleCopy() {
    if (lines.length === 0) return;
    const copied = await copyText(lines.join("\n"));
    setNotice(
      copied
        ? { tone: "success", text: `已复制 ${lines.length} 行` }
        : { tone: "warning", text: "复制失败，可改用「导出」" },
    );
  }

  function handleExport() {
    if (lines.length === 0) return;
    exportLogs.mutate(
      { lines, label: source === "file" ? (fileName ?? "file") : "memory" },
      {
        onSuccess: (result) =>
          setNotice({ tone: "success", text: `已导出 ${result.lines} 行到 ${result.path}` }),
        onError: (error) => setNotice({ tone: "error", text: `导出失败：${String(error)}` }),
      },
    );
  }

  function handleOpenDir() {
    openDir.mutate(undefined, {
      onError: (error) => setNotice({ tone: "error", text: String(error) }),
    });
  }

  const meta = [
    isLoading ? "读取中…" : `${lines.length} 行`,
    source === "memory" && memory.data ? `缓冲 ${memory.data.buffered} 行` : "",
    source === "file" && range
      ? `第 ${range.start + 1}–${range.end} 行 / 共 ${range.total} 行`
      : "",
    source === "file" && tail ? fmtBytes(tail.bytes) : "",
    keyword.trim() ? "已按关键字过滤" : "",
    level !== "all" ? "已按级别过滤" : "",
    source === "memory" && paused ? "已暂停" : "",
    source === "file" && !file.hasNextPage && range ? "已到文件开头" : "",
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <>
      <Title level={3} className="cg-page-title">
        日志
      </Title>
      <div className="cg-subtitle">
        <Text type="secondary">
          内存缓冲保留本次运行的最近 1000
          行；落盘日志按天轮转，可向上翻页。两者都只含元数据与错误原因，不含请求体和响应体。
        </Text>
      </div>

      <Space className="cg-toolbar" wrap>
        <Space.Compact>
          <Button
            type={source === "memory" ? "primary" : "default"}
            onClick={() => setSource("memory")}
          >
            内存
          </Button>
          <Button
            type={source === "file" ? "primary" : "default"}
            onClick={() => setSource("file")}
          >
            落盘文件
          </Button>
        </Space.Compact>

        {source === "file" ? (
          <Select
            className="cg-file-select"
            value={fileName ?? undefined}
            placeholder="暂无落盘文件"
            onChange={(value) => setFileName(value ?? null)}
            aria-label="选择日志文件"
            options={(files.data?.files ?? []).map((item) => ({
              value: item.name,
              label: `${item.name} · ${fmtBytes(item.bytes)}`,
            }))}
          />
        ) : null}

        <Select
          className="cg-level-select"
          value={level}
          onChange={setLevel}
          aria-label="日志级别"
          options={LEVELS}
        />

        <Input
          className="cg-input--search"
          allowClear
          placeholder="按关键字过滤"
          value={keyword}
          onChange={(event) => setKeyword(event.target.value)}
          aria-label="按关键字过滤日志"
        />

        {source === "memory" ? (
          <>
            <Checkbox checked={follow} onChange={(event) => setFollow(event.target.checked)}>
              自动滚动
            </Checkbox>
            <Button onClick={() => setPaused((value) => !value)}>
              {paused ? "继续刷新" : "暂停"}
            </Button>
          </>
        ) : (
          <>
            <span className="cg-toolbar__spacer" />
            <Button disabled={!canLoadEarlier} onClick={() => file.fetchNextPage()}>
              {file.isFetchingNextPage ? "加载中…" : "加载更早"}
            </Button>
            <Button onClick={() => file.refetch()}>刷新</Button>
          </>
        )}
      </Space>

      <Space className="cg-toolbar" wrap>
        <Button disabled={lines.length === 0} onClick={handleCopy}>
          复制
        </Button>
        <Button disabled={lines.length === 0 || exportLogs.isPending} onClick={handleExport}>
          {exportLogs.isPending ? "导出中…" : "导出"}
        </Button>
        <Button onClick={handleOpenDir}>打开目录</Button>
      </Space>

      <div className="cg-log-meta">
        <Text type="secondary">{meta}</Text>
      </div>

      {notice ? (
        <Alert className="cg-alert" type={notice.tone} showIcon message={notice.text} />
      ) : null}
      {levelState.data?.source === "env" ? (
        <Alert
          className="cg-alert"
          type="info"
          showIcon
          message={`当前级别由 RUST_LOG 指定（${levelState.data.level}），设置页的切换仍然优先。`}
        />
      ) : null}

      {loadError ? (
        <Alert
          className="cg-alert"
          type="error"
          showIcon
          message={`读取日志失败：${String(loadError)}`}
        />
      ) : source === "file" && files.data && files.data.files.length === 0 ? (
        <div className="cg-empty">
          {files.data.dir
            ? "日志目录里还没有文件，产生第一条日志后会出现"
            : "日志目录尚未就绪，稍后重试"}
        </div>
      ) : lines.length === 0 ? (
        <div className="cg-empty">{isLoading ? "读取中…" : "没有匹配的日志"}</div>
      ) : (
        <div className="cg-logbox" ref={boxRef}>
          {lines.map((line, index) => {
            const tone = levelOf(line);
            return (
              <div
                key={`${index}-${line.slice(0, 24)}`}
                className={`cg-log-line${tone ? ` cg-log-line--${tone}` : ""}`}
              >
                {line}
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
