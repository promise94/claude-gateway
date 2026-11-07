import { describe, expect, it } from "vitest";

import { fmtBytes, fmtClock, levelOf } from "./format";

describe("日志展示格式化", () => {
  it("fmtBytes 按量级切换单位", () => {
    expect(fmtBytes(512)).toBe("512B");
    expect(fmtBytes(2048)).toBe("2.0KB");
    expect(fmtBytes(3 * 1024 * 1024)).toBe("3.0MB");
  });

  it("levelOf 识别 tracing 输出的级别列", () => {
    expect(levelOf("2026-11-05 21:00:00.000  WARN rid=00a1f3 上游流提前结束")).toBe("WARN");
    expect(levelOf("2026-11-05 21:00:00.000 ERROR 启动失败")).toBe("ERROR");
    expect(levelOf("2026-11-05 21:00:00.000  INFO rid=00a1f3 POST /v1/messages")).toBe("INFO");
    expect(levelOf("没有级别的一行")).toBeNull();
  });

  it("fmtClock 只取时分秒", () => {
    expect(fmtClock("2026-11-05 21:03:11.123")).toBe("21:03:11");
  });
});
