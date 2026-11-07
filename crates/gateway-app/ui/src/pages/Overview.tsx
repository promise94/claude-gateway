import { useEffect } from "react";
import { Alert, Card, Col, Row, Statistic, Table, Tag, Typography, type TableProps } from "antd";

import { useStatus, useUpstreamProbe } from "../hooks/useGateway";
import { fmtBytes, fmtClock, fmtDuration, fmtMs, outcomeLabel, statusTone } from "../lib/format";
import type { RequestRow, StreamRow } from "../types";

const { Title, Text } = Typography;

type Columns<T> = TableProps<T>["columns"];

const toneColor: Record<ReturnType<typeof statusTone>, string> = {
  ok: "success",
  warn: "warning",
  danger: "error",
};

function averageUpstream(rows: { upstream_ms: number }[]): string {
  const withValue = rows.filter((row) => row.upstream_ms > 0);
  if (withValue.length === 0) return "—";
  const total = withValue.reduce((sum, row) => sum + row.upstream_ms, 0);
  return fmtMs(Math.round(total / withValue.length));
}

export function Overview() {
  const { data, isLoading, isError, error } = useStatus();
  // 端口绑定成功不等于上游可用：进页面探一次，之后每 30 秒复查
  const probe = useUpstreamProbe();
  const { mutate: probeUpstream } = probe;

  useEffect(() => {
    probeUpstream();
    const timer = setInterval(probeUpstream, 30_000);
    return () => clearInterval(timer);
  }, [probeUpstream]);

  if (isLoading) return <div className="cg-empty">正在读取状态…</div>;
  if (isError) return <div className="cg-empty cg-empty--error">读取状态失败：{String(error)}</div>;
  if (!data) return <div className="cg-empty">没有数据</div>;

  const { counters, requests, streams, rules } = data;
  const abnormal = streams.filter((stream) => stream.outcome !== "complete").slice(0, 5);

  const upstreamDown = probe.data ? !probe.data.reachable : false;
  // 地址取探针实际请求的那个：窗口失焦时 status 停止轮询，用 data.upstream 会显示旧值
  const upstreamState = probe.isPending
    ? "上游检测中"
    : probe.data
      ? probe.data.reachable
        ? `上游可达 ${fmtMs(probe.data.latency_ms)} · ${probe.data.url}`
        : `上游不可达 · ${probe.data.url} · ${probe.data.error ?? "未知原因"}`
      : `上游 ${data.upstream}`;

  const requestColumns: Columns<RequestRow> = [
    {
      title: "时间",
      dataIndex: "at",
      render: (at: string) => <span className="cg-mono">{fmtClock(at)}</span>,
    },
    {
      title: "ID",
      dataIndex: "rid",
      render: (rid: string) => <span className="cg-mono cg-muted">{rid}</span>,
    },
    {
      title: "路径",
      dataIndex: "path",
      render: (path: string) => <span className="cg-path">{path}</span>,
    },
    {
      title: "状态",
      dataIndex: "status",
      render: (status: number) => <Tag color={toneColor[statusTone(status)]}>{status}</Tag>,
    },
    {
      title: "总耗时",
      dataIndex: "elapsed_ms",
      render: (ms: number) => <span className="cg-mono">{fmtMs(ms)}</span>,
    },
    {
      title: "上游",
      dataIndex: "upstream_ms",
      render: (ms: number) => <span className="cg-mono">{fmtMs(ms)}</span>,
    },
    {
      title: "命中规则",
      dataIndex: "rewritten",
      render: (value: string) => <span className="cg-mono cg-muted">{value}</span>,
    },
  ];

  const streamColumns: Columns<StreamRow> = [
    {
      title: "时间",
      dataIndex: "at",
      render: (at: string) => <span className="cg-mono">{fmtClock(at)}</span>,
    },
    {
      title: "ID",
      dataIndex: "rid",
      render: (rid: string) => <span className="cg-mono cg-muted">{rid}</span>,
    },
    {
      title: "结果",
      dataIndex: "outcome",
      render: (outcome: StreamRow["outcome"]) => (
        <Tag color={outcome === "complete" ? "success" : "warning"}>{outcomeLabel(outcome)}</Tag>
      ),
    },
    {
      title: "字节",
      dataIndex: "bytes",
      render: (bytes: number) => <span className="cg-mono">{fmtBytes(bytes)}</span>,
    },
    {
      title: "块数",
      dataIndex: "chunks",
      render: (chunks: number) => <span className="cg-mono">{chunks}</span>,
    },
    {
      title: "耗时",
      dataIndex: "elapsed_ms",
      render: (ms: number) => <span className="cg-mono">{fmtMs(ms)}</span>,
    },
  ];

  return (
    <>
      <Title level={3} className="cg-page-title">
        总览
      </Title>
      <div className="cg-subtitle">
        <Text type="secondary">
          本机网关的实时状态。数据每 2 秒刷新一次，全部来自内存遥测，不读取请求体。
        </Text>
      </div>

      <Alert
        className="cg-alert cg-alert--status"
        showIcon
        type={!data.running ? "error" : upstreamDown ? "warning" : "success"}
        message={data.running ? `转发已就绪，监听 ${data.listen}` : "网关未启动"}
        description={
          data.listen_error
            ? `原因：${data.listen_error}`
            : `${upstreamState} · 运行 ${fmtDuration(data.uptime_secs)} · pid ${data.pid}`
        }
      />

      {upstreamDown ? (
        <Alert
          className="cg-alert"
          type="warning"
          showIcon
          message="连不上上游，转发会返回 502。到「设置」页确认上游地址。"
        />
      ) : null}

      <Row gutter={[12, 12]} className="cg-metric-grid">
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic title="请求总数" value={counters.requests} />
            <div className="cg-card__desc">自启动累计</div>
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="错误响应"
              value={counters.errors}
              valueStyle={counters.errors > 0 ? { color: "var(--cg-danger)" } : undefined}
            />
            <div className="cg-card__desc">状态码 ≥ 400</div>
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="流异常"
              value={counters.stream_truncated + counters.stream_errors}
              valueStyle={
                counters.stream_truncated + counters.stream_errors > 0
                  ? { color: "var(--cg-warning)" }
                  : undefined
              }
            />
            <div className="cg-card__desc">
              提前结束 {counters.stream_truncated} · 中断 {counters.stream_errors}
            </div>
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic title="上游首字节" value={averageUpstream(requests)} />
            <div className="cg-card__desc">最近 {requests.length} 条均值</div>
          </Card>
        </Col>
      </Row>

      <Card
        className="cg-section-card"
        size="small"
        title="规则命中"
        extra={`${rules.filter((rule) => rule.enabled).length} 条启用`}
      >
        {rules.map((rule) => (
          <div className="cg-rule-summary__row" key={rule.id}>
            <span className={`cg-dot ${rule.enabled ? "cg-dot--success" : ""}`} />
            <span className="cg-rule-summary__label">{rule.label}</span>
            <span className="cg-rule-summary__hits">{rule.hits}</span>
          </div>
        ))}
      </Card>

      <Card
        className="cg-section-card"
        size="small"
        title="最近请求"
        extra={`${requests.length} 条`}
      >
        {requests.length === 0 ? (
          <div className="cg-empty cg-empty--inline">还没有请求经过网关</div>
        ) : (
          <Table<RequestRow>
            rowKey="rid"
            size="small"
            pagination={false}
            scroll={{ x: "max-content" }}
            dataSource={requests.slice(0, 8)}
            columns={requestColumns}
          />
        )}
      </Card>

      <Card className="cg-section-card" size="small" title="流异常" extra={`${abnormal.length} 条`}>
        {abnormal.length === 0 ? (
          <div className="cg-empty cg-empty--inline">最近没有流异常</div>
        ) : (
          <Table<StreamRow>
            rowKey="rid"
            size="small"
            pagination={false}
            scroll={{ x: "max-content" }}
            dataSource={abnormal}
            columns={streamColumns}
          />
        )}
      </Card>
    </>
  );
}
