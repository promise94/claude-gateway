import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { TokenSwatch } from "./TokenSwatch";

describe("TokenSwatch", () => {
  it("展示标签、色值与色块", () => {
    render(<TokenSwatch name="accent" label="主色 赤陶" />);
    expect(screen.getByText("主色 赤陶")).toBeInTheDocument();
    expect(screen.getByText("#B9533A")).toBeInTheDocument();
  });

  it("色块使用传入 token 的颜色", () => {
    const { container } = render(<TokenSwatch name="success" label="成功" />);
    const chip = container.querySelector<HTMLElement>(".cg-swatch__chip");
    // jsdom 会把十六进制色值规范化为 rgb()，两种写法都接受
    expect(chip?.style.background ?? "").toMatch(/^(#6B7F5B|rgb\(107, 127, 91\))$/i);
  });
});
