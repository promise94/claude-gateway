import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ipc } from "../lib/ipc";
import { Logs } from "./Logs";

// 组件只通过 ipc 取数，这里整体替掉，不碰 Tauri
vi.mock("../lib/ipc", () => ({
  ipc: {
    logs: vi.fn(),
    logFiles: vi.fn(),
    logFile: vi.fn(),
    logLevel: vi.fn(),
    logLevelSet: vi.fn(),
    exportLogs: vi.fn(),
    openLogDir: vi.fn(),
  },
}));

const FILE = "gateway.2026-11-04.log";

function renderLogs() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <Logs />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.mocked(ipc.logLevel).mockResolvedValue({
    level: "info",
    source: "config",
    config_level: "info",
    options: ["trace", "debug", "info", "warn", "error"],
  });
  vi.mocked(ipc.logFiles).mockResolvedValue({ dir: "/tmp/logs", files: [] });
  vi.mocked(ipc.logs).mockResolvedValue({
    lines: ["2026-11-05 21:00:00.000  INFO rid=000001 内存里的一行"],
    buffered: 1,
    source: "memory",
  });
});

describe("日志页", () => {
  it("默认展示内存缓冲", async () => {
    renderLogs();

    expect(await screen.findByText(/内存里的一行/)).toBeInTheDocument();
    expect(screen.getByText(/缓冲 1 行/)).toBeInTheDocument();
  });

  it("切到落盘文件后按天读取并显示范围", async () => {
    vi.mocked(ipc.logFiles).mockResolvedValue({
      dir: "/tmp/logs",
      files: [{ name: FILE, bytes: 2048, modified: 0 }],
    });
    vi.mocked(ipc.logFile).mockResolvedValue({
      name: FILE,
      lines: ["file-line-a", "file-line-b"],
      total: 2,
      start: 0,
      end: 2,
      has_more: false,
      bytes: 2048,
      source: "file",
    });

    renderLogs();
    fireEvent.click(screen.getByRole("button", { name: "落盘文件" }));

    expect(await screen.findByText("file-line-a")).toBeInTheDocument();
    expect(screen.getByText(/共 2 行/)).toBeInTheDocument();
    // 默认选中最新一天，不需要手动挑文件
    expect(vi.mocked(ipc.logFile)).toHaveBeenCalledWith(FILE, undefined, 400);
  });

  it("加载更早时把上一页拼到前面", async () => {
    vi.mocked(ipc.logFiles).mockResolvedValue({
      dir: "/tmp/logs",
      files: [{ name: FILE, bytes: 4096, modified: 0 }],
    });
    vi.mocked(ipc.logFile).mockImplementation(async (_name, before) => {
      if (before === undefined) {
        return {
          name: FILE,
          lines: ["tail-9"],
          total: 10,
          start: 9,
          end: 10,
          has_more: true,
          bytes: 4096,
          source: "file",
        };
      }
      return {
        name: FILE,
        lines: ["head-0"],
        total: 10,
        start: 0,
        end: 9,
        has_more: false,
        bytes: 4096,
        source: "file",
      };
    });

    const { container } = renderLogs();
    fireEvent.click(screen.getByRole("button", { name: "落盘文件" }));
    await screen.findByText("tail-9");

    fireEvent.click(screen.getByRole("button", { name: "加载更早" }));
    await screen.findByText("head-0");

    // pages 反序渲染：更早的行必须在前面
    const text = container.textContent ?? "";
    expect(text.indexOf("head-0")).toBeLessThan(text.indexOf("tail-9"));
  });

  it("没有落盘文件时给出空态而不是报错", async () => {
    renderLogs();
    fireEvent.click(screen.getByRole("button", { name: "落盘文件" }));

    expect(await screen.findByText(/还没有文件/)).toBeInTheDocument();
  });
});
