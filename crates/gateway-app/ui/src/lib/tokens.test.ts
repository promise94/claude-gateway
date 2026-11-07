import { describe, expect, it } from "vitest";
import { colorTokens, cssVar } from "./tokens";

describe("colorTokens", () => {
  it("C 暖米白 + 赤陶的关键色值与设计基线一致", () => {
    expect(colorTokens.surface).toBe("#F7F4EE");
    expect(colorTokens.sidebar).toBe("#F1ECE2");
    expect(colorTokens.panel).toBe("#FFFDF8");
    expect(colorTokens.text).toBe("#2B2722");
    expect(colorTokens.border).toBe("#E5DED2");
    expect(colorTokens.accent).toBe("#B9533A");
    expect(colorTokens.accentSoft).toBe("#F7EAE4");
    expect(colorTokens.success).toBe("#6B7F5B");
    expect(colorTokens.danger).toBe("#A33B2A");
    expect(colorTokens.warning).toBe("#B8863B");
  });

  it("每个 token 都是合法的十六进制色值", () => {
    for (const value of Object.values(colorTokens)) {
      expect(value).toMatch(/^#[0-9A-F]{6}$/);
    }
  });

  it("cssVar 把驼峰名转成 CSS 连字符变量名", () => {
    expect(cssVar("accent")).toBe("var(--cg-accent)");
    expect(cssVar("accentSoft")).toBe("var(--cg-accent-soft)");
    expect(cssVar("textMuted")).toBe("var(--cg-text-muted)");
  });
});
