/**
 * 已确认的设计基线：C 暖米白 + 赤陶。
 * 色值与 src/styles/tokens.css 的变量一一对应；改动前先更新 docs/decisions.md D16。
 */
export const colorTokens = {
  surface: "#F7F4EE",
  topbar: "#FBF9F4",
  sidebar: "#F1ECE2",
  panel: "#FFFDF8",
  text: "#2B2722",
  textMuted: "#8A8175",
  textFaint: "#B4AA9A",
  border: "#E5DED2",
  accent: "#B9533A",
  accentSoft: "#F7EAE4",
  success: "#6B7F5B",
  danger: "#A33B2A",
  warning: "#B8863B",
  onAccent: "#FFF9F3",
} as const;

export type ColorToken = keyof typeof colorTokens;

/** 返回对应的 CSS 变量引用，组件不要直接写死色值。驼峰名转为 CSS 的连字符写法。 */
export function cssVar(name: ColorToken): string {
  const kebab = name.replace(/[A-Z]/g, (char) => `-${char.toLowerCase()}`);
  return `var(--cg-${kebab})`;
}
