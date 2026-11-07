import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { ipc } from "../lib/ipc";

/** 状态快照，2 秒一轮。桌面端窗口失焦时停轮询，避免无意义的 IPC。 */
export function useStatus() {
  return useQuery({
    queryKey: ["status"],
    queryFn: ipc.status,
    refetchInterval: 2000,
    refetchIntervalInBackground: false,
  });
}

/** 日志：切级别会换 queryKey 立即重取，暂停时停轮询 */
export function useLogs(level: string, paused: boolean) {
  return useQuery({
    queryKey: ["logs", level],
    queryFn: () => ipc.logs(400, level === "all" ? undefined : level),
    refetchInterval: paused ? false : 2000,
  });
}

/** 落盘文件清单。只在切到文件模式时启用，轮转一天一次，10 秒一轮足够。 */
export function useLogFiles(enabled: boolean) {
  return useQuery({
    queryKey: ["log-files"],
    queryFn: ipc.logFiles,
    enabled,
    refetchInterval: enabled ? 10_000 : false,
  });
}

/**
 * 文件日志分页。
 *
 * 第一页从文件末尾往前取，`getNextPageParam` 返回当前页的 `start`，
 * 继续翻就是更早的内容。渲染时要把 pages 反序，否则「加载更早」会跑到下面。
 */
export function useLogFile(name: string | null, pageSize = 400) {
  return useInfiniteQuery({
    queryKey: ["log-file", name, pageSize],
    queryFn: ({ pageParam }) => {
      if (!name) throw new Error("未选择日志文件");
      return ipc.logFile(name, pageParam, pageSize);
    },
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => (last.has_more ? last.start : undefined),
    enabled: Boolean(name),
    staleTime: 5_000,
  });
}

export function useLogLevel() {
  return useQuery({
    queryKey: ["log-level"],
    queryFn: ipc.logLevel,
    staleTime: 30_000,
  });
}

export function useLogLevelSet() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (level: string) => ipc.logLevelSet(level),
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ["log-level"] });
      client.invalidateQueries({ queryKey: ["config"] });
      // 级别变了，内存日志能看到的范围也变了
      client.invalidateQueries({ queryKey: ["logs"] });
    },
  });
}

export function useExportLogs() {
  return useMutation({
    mutationFn: (input: { lines: string[]; label?: string }) =>
      ipc.exportLogs(input.lines, input.label),
  });
}

export function useOpenLogDir() {
  return useMutation({ mutationFn: ipc.openLogDir });
}

export function useConfig() {
  return useQuery({
    queryKey: ["config"],
    queryFn: ipc.config,
    staleTime: 30_000,
  });
}

export function useRuleUpdate() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: { id: string; enabled: boolean; params: Record<string, string> }) =>
      ipc.ruleUpdate(input.id, input.enabled, input.params),
    onSuccess: () => client.invalidateQueries({ queryKey: ["status"] }),
  });
}

export function useRuleSave() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ipc.ruleSave,
    onSuccess: () => client.invalidateQueries({ queryKey: ["status"] }),
  });
}

export function useRuleDiscard() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ipc.ruleDiscard,
    onSuccess: () => client.invalidateQueries({ queryKey: ["status"] }),
  });
}

export function useUpstreamProbe() {
  return useMutation({ mutationFn: ipc.upstream });
}

/** 改上游地址：立即生效并写回配置，成功后刷新配置与状态 */
export function useUpstreamSet() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (upstream: string) => ipc.upstreamSet(upstream),
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ["config"] });
      client.invalidateQueries({ queryKey: ["status"] });
    },
  });
}
