import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

// 没有开 globals，@testing-library/react 不会自动注册 afterEach 清理；
// 不显式 cleanup 的话，上一个用例的 DOM 会留到下一个，getByRole 会命中重复元素。
afterEach(() => {
  cleanup();
});
