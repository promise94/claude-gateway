import "antd/dist/reset.css";
import { ConfigProvider } from "antd";
import zhCN from "antd/locale/zh_CN";
import { Navigate, Route, Routes } from "react-router-dom";

import { useStatus } from "./hooks/useGateway";
import { Sidebar } from "./layout/Sidebar";
import { colorTokens } from "./lib/tokens";
import { Logs } from "./pages/Logs";
import { Overview } from "./pages/Overview";
import { Rules } from "./pages/Rules";
import { Settings } from "./pages/Settings";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/app.css";

/**
 * antd 主题映射到已确认的 C 暖米白 + 赤陶设计基线（docs/decisions.md D16）。
 * 色值统一取自 lib/tokens.ts，组件里不写死颜色。
 */
const cgTheme = {
  token: {
    colorPrimary: colorTokens.accent,
    colorBgBase: colorTokens.surface,
    colorBgContainer: colorTokens.panel,
    colorBgLayout: colorTokens.surface,
    colorBgElevated: colorTokens.panel,
    colorText: colorTokens.text,
    colorTextSecondary: colorTokens.textMuted,
    colorTextTertiary: colorTokens.textFaint,
    colorBorder: colorTokens.border,
    colorBorderSecondary: colorTokens.border,
    colorSuccess: colorTokens.success,
    colorError: colorTokens.danger,
    colorWarning: colorTokens.warning,
    colorInfo: colorTokens.accent,
    colorLink: colorTokens.accent,
    borderRadius: 8,
    borderRadiusLG: 12,
    borderRadiusSM: 5,
    fontSize: 13,
    controlHeight: 30,
    fontFamily:
      "-apple-system, BlinkMacSystemFont, 'SF Pro Text', 'PingFang SC', 'Helvetica Neue', sans-serif",
  },
};

export default function App() {
  // 侧栏要用运行状态，这里只取一次；页面内部的同名 query 会复用缓存，不会重复请求
  const { data } = useStatus();

  return (
    <ConfigProvider locale={zhCN} theme={cgTheme}>
      <div className="cg-app">
        <Sidebar status={data} />
        <main className="cg-main">
          <Routes>
            <Route path="/" element={<Overview />} />
            <Route path="/logs" element={<Logs />} />
            <Route path="/rules" element={<Rules />} />
            <Route path="/settings" element={<Settings />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </main>
      </div>
    </ConfigProvider>
  );
}
