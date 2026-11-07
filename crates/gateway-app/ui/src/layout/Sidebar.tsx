import {
  ControlOutlined,
  DashboardOutlined,
  FileTextOutlined,
  SettingOutlined,
} from "@ant-design/icons";
import { Menu, Tag, type MenuProps } from "antd";
import { useLocation, useNavigate } from "react-router-dom";

import { fmtDuration } from "../lib/format";
import type { Status } from "../types";

interface Props {
  status?: Status;
}

export function Sidebar({ status }: Props) {
  const location = useLocation();
  const navigate = useNavigate();
  const running = status?.running ?? false;
  const dirty = status?.rules_dirty ?? false;

  const items: MenuProps["items"] = [
    {
      type: "group",
      label: "监控",
      children: [
        { key: "/", icon: <DashboardOutlined />, label: "总览" },
        { key: "/logs", icon: <FileTextOutlined />, label: "日志" },
      ],
    },
    {
      type: "group",
      label: "规则工作台",
      children: [
        {
          key: "/rules",
          icon: <ControlOutlined />,
          label: dirty ? (
            <span>
              兼容规则{" "}
              <Tag color="orange" style={{ marginInlineEnd: 0 }}>
                未保存
              </Tag>
            </span>
          ) : (
            "兼容规则"
          ),
        },
        { key: "/settings", icon: <SettingOutlined />, label: "设置" },
      ],
    },
  ];

  return (
    <aside className="cg-sidebar">
      <div className="cg-brand">
        {/* 与 App 图标同款的适配器图形，深底版本适配浅色侧栏 */}
        <svg className="cg-brand__mark" viewBox="0 0 64 64" role="img" aria-label="Claude Gateway">
          <rect width="64" height="64" rx="14" fill="var(--cg-text)" />
          <rect x="25" y="29" width="14" height="6" fill="var(--cg-accent)" />
          <circle cx="20" cy="32" r="8" fill="var(--cg-on-accent)" />
          <rect x="36" y="24" width="16" height="16" rx="3" fill="var(--cg-on-accent)" />
        </svg>
        <span>
          Claude Gateway
          <span className="cg-brand__version">v{status?.version ?? "0.1.0"}</span>
        </span>
      </div>

      <Menu
        className="cg-nav"
        mode="inline"
        selectedKeys={[location.pathname]}
        items={items}
        onClick={({ key }) => navigate(key)}
      />

      <div className="cg-sidebar__footer">
        <span className={`cg-dot ${running ? "cg-dot--success" : "cg-dot--danger"}`} />
        <span className="cg-sidebar__state">
          {running ? "运行中" : "未启动"}
          {status ? ` · ${fmtDuration(status.uptime_secs)}` : ""}
        </span>
      </div>
    </aside>
  );
}
