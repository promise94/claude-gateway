/** 展示层格式化。所有函数对空值给可读占位，不抛异常。 */

export type LogLevel = "ERROR" | "WARN" | "INFO" | "DEBUG" | null;

export function fmtDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "—";
  const seconds = Math.floor(totalSeconds);
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${seconds % 60}s`;
  return `${seconds}s`;
}

/** 0 是真实值（本机环回常见），只有缺失才显示占位 */
export function fmtMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "—";
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

export function fmtBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)}KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)}MB`;
}

/** "2026-11-05 21:03:11.123" → "21:03:11"；解析不出来就原样返回 */
export function fmtClock(at: string): string {
  const match = at.match(/(\d{2}:\d{2}:\d{2})/);
  return match ? match[1] : at;
}

export function levelOf(line: string): LogLevel {
  if (/\sERROR\s/.test(line)) return "ERROR";
  if (/\sWARN\s/.test(line)) return "WARN";
  if (/\sINFO\s/.test(line)) return "INFO";
  if (/\sDEBUG\s/.test(line)) return "DEBUG";
  return null;
}

export function outcomeLabel(outcome: string): string {
  switch (outcome) {
    case "complete":
      return "完成";
    case "truncated":
      return "提前结束";
    case "error":
      return "中断";
    case "client_closed":
      return "客户端断开";
    default:
      return outcome;
  }
}

/** 状态码语义：2xx 正常、4xx 警告、5xx 错误 */
export function statusTone(status: number): "ok" | "warn" | "danger" {
  if (status >= 500) return "danger";
  if (status >= 400) return "warn";
  return "ok";
}
