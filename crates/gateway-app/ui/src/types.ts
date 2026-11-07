/** 与 gateway-core 的 panel 快照一一对应。改结构时两边一起改。 */

export interface RuleParam {
  key: string;
  label: string;
  default: string;
  required_placeholder: string | null;
  value: string;
}

export interface Rule {
  id: string;
  label: string;
  description: string;
  enabled: boolean;
  hits: number;
  params: RuleParam[];
}

export interface Counters {
  requests: number;
  errors: number;
  streams: number;
  stream_truncated: number;
  stream_errors: number;
  client_closed: number;
}

export interface RequestRow {
  at: string;
  /** 6 位短 ID，与日志里的 rid= 对应 */
  rid: string;
  method: string;
  path: string;
  status: number;
  elapsed_ms: number;
  upstream_ms: number;
  rewritten: string;
}

export type StreamOutcome = "complete" | "truncated" | "error" | "client_closed";

export interface StreamRow {
  at: string;
  rid: string;
  path: string;
  outcome: StreamOutcome;
  bytes: number;
  chunks: number;
  elapsed_ms: number;
  detail: string | null;
}

export interface Status {
  version: string;
  pid: number;
  uptime_secs: number;
  started_at: number;
  listen: string;
  upstream: string;
  config_source: string | null;
  log_dir: string | null;
  log_buffered: number;
  counters: Counters;
  rules: Rule[];
  rules_dirty: boolean;
  requests: RequestRow[];
  streams: StreamRow[];
  running: boolean;
  listen_error: string | null;
}

export interface LogsResponse {
  lines: string[];
  buffered: number;
  source: string;
}

export interface LogFileEntry {
  name: string;
  bytes: number;
  /** unix 秒 */
  modified: number;
}

export interface LogFilesResponse {
  /** null 表示日志目录尚未就绪（日志系统未完成初始化） */
  dir: string | null;
  files: LogFileEntry[];
}

export interface LogFileResponse {
  name: string;
  lines: string[];
  total: number;
  /** 本页第一行的下标（0 基） */
  start: number;
  /** 本页结束下标（不含） */
  end: number;
  has_more: boolean;
  bytes: number;
  source: string;
}

/** config：来自 config.toml；env：被 RUST_LOG 覆盖；runtime：本次运行期切换过 */
export type LogLevelSource = "config" | "env" | "runtime";

export interface LogLevelState {
  level: string;
  source: LogLevelSource;
  config_level: string;
  options: string[];
  /** 仅切换接口返回：是否已写回配置文件 */
  persisted?: boolean;
  /** 仅切换接口返回：写盘失败原因 */
  persist_error?: string | null;
}

export interface ExportResult {
  path: string;
  dir: string;
  bytes: number;
  lines: number;
}

/** existing：已有；migrated：从旧位置迁移；created：本次新建；env：环境变量指定；builtin：内置默认 */
export type ConfigOriginKind = "existing" | "migrated" | "created" | "env" | "builtin";

export interface ConfigSnapshot {
  source: string | null;
  origin: ConfigOriginKind;
  origin_text: string;
  base_dir: string;
  listen: string;
  upstream: string;
  max_body_bytes: number;
  use_system_proxy: boolean;
  log: {
    /** config.toml 里的值 */
    level: string;
    /** 当前进程实际生效的值 */
    runtime_level: string;
    level_source: LogLevelSource;
    dir: string | null;
    retain_days: number;
  };
  unregistered_rules: string[];
}

export interface UpstreamProbe {
  url: string;
  reachable: boolean;
  status: number | null;
  latency_ms: number;
  error: string | null;
}

export interface UpstreamSetResult {
  upstream: string;
  persisted: boolean;
  persist_error: string | null;
}

export interface RuleUpdateResult {
  rule: Rule;
  dirty: boolean;
  /** 是否已写进配置文件 */
  persisted: boolean;
  /** 写盘失败原因，成功时为 null */
  save_error: string | null;
}

export interface RuleSaveResult {
  saved: boolean;
  path: string;
  dirty: boolean;
}

export interface RuleDiscardResult {
  dirty: boolean;
  rules: Rule[];
}
