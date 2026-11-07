import { invoke } from "@tauri-apps/api/core";

import type {
  ConfigSnapshot,
  ExportResult,
  LogFileResponse,
  LogFilesResponse,
  LogLevelState,
  LogsResponse,
  RuleDiscardResult,
  RuleSaveResult,
  RuleUpdateResult,
  Status,
  UpstreamProbe,
  UpstreamSetResult,
} from "../types";

/** 组件不直接调 invoke：命令名和参数集中在这里，改后端时只改一处。 */
export const ipc = {
  status: () => invoke<Status>("status"),

  logs: (limit: number, level?: string) => invoke<LogsResponse>("logs", { limit, level }),

  logFiles: () => invoke<LogFilesResponse>("log_files"),

  /** before 不传表示取文件末尾；传上一页的 start 就是向上翻 */
  logFile: (name: string, before?: number, limit?: number) =>
    invoke<LogFileResponse>("log_file", { name, before, limit }),

  logLevel: () => invoke<LogLevelState>("log_level"),

  logLevelSet: (level: string) => invoke<LogLevelState>("log_level_set", { level }),

  exportLogs: (lines: string[], label?: string) =>
    invoke<ExportResult>("export_logs", { lines, label }),

  openLogDir: () => invoke<string>("open_log_dir"),

  config: () => invoke<ConfigSnapshot>("config"),

  upstream: () => invoke<UpstreamProbe>("upstream"),

  upstreamSet: (upstream: string) => invoke<UpstreamSetResult>("upstream_set", { upstream }),

  ruleUpdate: (id: string, enabled: boolean, params: Record<string, string>) =>
    invoke<RuleUpdateResult>("rule_update", { id, enabled, params }),

  ruleSave: () => invoke<RuleSaveResult>("rule_save"),

  ruleDiscard: () => invoke<RuleDiscardResult>("rule_discard"),
};
