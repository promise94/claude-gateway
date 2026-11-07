import { useState } from "react";
import {
  Alert,
  Button,
  Descriptions,
  Input,
  Segmented,
  Space,
  Tag,
  Typography,
  type DescriptionsProps,
} from "antd";

import {
  useConfig,
  useLogLevel,
  useLogLevelSet,
  useUpstreamProbe,
  useUpstreamSet,
} from "../hooks/useGateway";
import { fmtMs } from "../lib/format";
import type { LogLevelSource } from "../types";

const { Title, Text } = Typography;

const LEVEL_HINTS: Record<string, string> = {
  trace: "最详细，第三方库也会打日志，量最大",
  debug: "排查用，含改写命中与流结束判定",
  info: "默认，每个转发请求一行访问日志",
  warn: "只看异常与告警",
  error: "只看错误",
};

const LEVEL_FALLBACK = ["trace", "debug", "info", "warn", "error"];

type Notice = { tone: "success" | "warning" | "error"; text: string } | null;

function levelSourceLabel(source: LogLevelSource): string {
  switch (source) {
    case "env":
      return "被 RUST_LOG 覆盖";
    case "runtime":
      return "运行期切换";
    default:
      return "与配置文件一致";
  }
}

export function Settings() {
  const { data, isLoading, isError, error } = useConfig();
  const probe = useUpstreamProbe();
  const saveUpstream = useUpstreamSet();
  const levelState = useLogLevel();
  const setLevel = useLogLevelSet();

  // 输入框留空表示"跟随服务端的值"，保存成功后清回 null
  const [upstreamDraft, setUpstreamDraft] = useState<string | null>(null);
  const [upstreamNotice, setUpstreamNotice] = useState<Notice>(null);
  const [levelNotice, setLevelNotice] = useState<Notice>(null);
  // 首次生成的默认配置提示，用户保存过一次上游后就不再显示
  const [firstRunDone, setFirstRunDone] = useState(false);

  if (isLoading) return <div className="cg-empty">正在读取配置…</div>;
  if (isError) return <div className="cg-empty cg-empty--error">读取配置失败：{String(error)}</div>;
  if (!data) return <div className="cg-empty">没有配置数据</div>;

  const options = levelState.data?.options ?? LEVEL_FALLBACK;
  const activeLevel = levelState.data?.level ?? data.log.runtime_level;
  const levelSource = levelState.data?.source ?? data.log.level_source;

  const upstream = upstreamDraft ?? data.upstream;
  const upstreamChanged = upstream.trim() !== data.upstream;

  function handleSaveUpstream() {
    saveUpstream.mutate(upstream.trim(), {
      onSuccess: (result) => {
        setUpstreamDraft(null);
        setFirstRunDone(true);
        setUpstreamNotice(
          result.persisted
            ? { tone: "success", text: `已切换到 ${result.upstream} 并写入 config.toml` }
            : {
                tone: "error",
                text: `已切换到 ${result.upstream}，但写入配置失败：${result.persist_error ?? "未知原因"}`,
              },
        );
      },
      onError: (mutationError) => setUpstreamNotice({ tone: "error", text: String(mutationError) }),
    });
  }

  const configRows: DescriptionsProps["items"] = [
    { key: "listen", label: "监听地址", children: <span className="cg-mono">{data.listen}</span> },
    {
      key: "source",
      label: "配置文件",
      children: <span className="cg-mono">{data.source ?? "—"}</span>,
    },
    {
      key: "origin",
      label: "配置来源",
      children: <span className="cg-mono">{data.origin_text}</span>,
    },
    {
      key: "base",
      label: "解析基准目录",
      children: <span className="cg-mono">{data.base_dir}</span>,
    },
    {
      key: "maxBody",
      label: "请求体上限",
      children: (
        <span className="cg-mono">{(data.max_body_bytes / 1024 / 1024).toFixed(0)} MiB</span>
      ),
    },
    {
      key: "proxy",
      label: "系统代理",
      children: <span className="cg-mono">{data.use_system_proxy ? "启用" : "禁用"}</span>,
    },
    {
      key: "levelConfig",
      label: "日志级别（配置）",
      children: <span className="cg-mono">{data.log.level}</span>,
    },
    {
      key: "levelRuntime",
      label: "日志级别（生效）",
      children: (
        <span className="cg-mono">
          {activeLevel} · {levelSourceLabel(levelSource)}
        </span>
      ),
    },
    {
      key: "logDir",
      label: "日志目录",
      children: <span className="cg-mono">{data.log.dir ?? "—"}</span>,
    },
    {
      key: "retain",
      label: "日志保留",
      children: <span className="cg-mono">{data.log.retain_days} 个文件</span>,
    },
  ];

  return (
    <>
      <Title level={3} className="cg-page-title">
        设置
      </Title>
      <div className="cg-subtitle">
        <Text type="secondary">
          上游地址与日志级别可以直接在这里改，改动立即生效并写回
          config.toml；监听地址、请求体上限等其余配置只读，需要手改文件后重启。
        </Text>
      </div>

      {data.origin === "created" && !firstRunDone ? (
        <Alert
          className="cg-alert"
          type="warning"
          showIcon
          message={`首次启动生成了默认配置，当前上游是 ${data.upstream}。如果这不是你的上游地址，改好下面的输入框并保存。`}
        />
      ) : null}

      <div className="cg-section-heading">
        <Text strong>上游地址</Text>
        <Text type="secondary">保存后下一个请求立即生效</Text>
      </div>
      <Space className="cg-toolbar" wrap>
        <Input
          className="cg-input--url"
          value={upstream}
          spellCheck={false}
          placeholder="http://127.0.0.1:15721"
          onChange={(event) => setUpstreamDraft(event.target.value)}
          aria-label="上游地址"
        />
        <Button
          type="primary"
          disabled={!upstreamChanged || saveUpstream.isPending}
          onClick={handleSaveUpstream}
        >
          {saveUpstream.isPending ? "保存中…" : "保存"}
        </Button>
        <Button disabled={probe.isPending} onClick={() => probe.mutate()}>
          {probe.isPending ? "探测中…" : "探测"}
        </Button>
        {probe.data ? (
          <Tag color={probe.data.reachable ? "success" : "error"}>
            {probe.data.reachable
              ? `可达 · HTTP ${probe.data.status ?? "?"} · ${fmtMs(probe.data.latency_ms)}`
              : `不可达 · ${probe.data.error ?? "未知原因"}`}
          </Tag>
        ) : null}
        {probe.error ? <Tag color="error">探测失败：{String(probe.error)}</Tag> : null}
      </Space>
      {upstreamNotice ? (
        <Alert
          className="cg-alert"
          type={upstreamNotice.tone}
          showIcon
          message={upstreamNotice.text}
        />
      ) : null}
      <div className="cg-hint">
        <Text type="secondary">
          写回时保留配置文件里的注释。探测只发 HEAD
          请求，用于区分「连不上上游」和「上游返回了错误状态码」。
        </Text>
      </div>

      <div className="cg-section-heading">
        <Text strong>日志级别</Text>
        <Text type="secondary">立即生效并写回配置</Text>
      </div>
      <Space className="cg-toolbar" wrap>
        <Segmented
          value={activeLevel}
          disabled={setLevel.isPending}
          options={options.map((item) => ({ label: item, value: item }))}
          onChange={(value) =>
            setLevel.mutate(String(value), {
              onSuccess: (result) =>
                setLevelNotice(
                  result.persisted
                    ? { tone: "success", text: `已切换到 ${result.level} 并写入 config.toml` }
                    : {
                        tone: "error",
                        text: `已切换到 ${result.level}，但写入配置失败：${result.persist_error ?? "未知原因"}`,
                      },
                ),
              onError: (mutationError) =>
                setLevelNotice({
                  tone: "error",
                  text: `切换失败：${String(mutationError)}`,
                }),
            })
          }
        />
        <Text type="secondary">{LEVEL_HINTS[activeLevel] ?? ""}</Text>
      </Space>
      {levelNotice ? (
        <Alert className="cg-alert" type={levelNotice.tone} showIcon message={levelNotice.text} />
      ) : null}
      <div className="cg-hint">
        <Text type="secondary">
          配置文件里是 {data.log.level}，当前进程生效 {activeLevel}（{levelSourceLabel(levelSource)}
          ）。改级别只影响日志多少，不影响转发行为。
        </Text>
      </div>

      <div className="cg-section-heading">
        <Text strong>生效配置</Text>
        <Text type="secondary">{data.source ?? "内置默认值"}</Text>
      </div>
      <Descriptions
        className="cg-descriptions"
        size="small"
        bordered
        column={1}
        items={configRows}
      />

      {data.unregistered_rules.length > 0 ? (
        <Alert
          className="cg-alert"
          type="error"
          showIcon
          message={`配置里有未注册的规则 id：${data.unregistered_rules.join(", ")}。通常是拼写错误，这些配置不会生效。`}
        />
      ) : null}
    </>
  );
}
