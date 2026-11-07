import { useState } from "react";
import { Alert, Button, Card, Input, Space, Switch, Tag, Typography } from "antd";

import { useRuleDiscard, useRuleUpdate, useStatus } from "../hooks/useGateway";
import type { Rule } from "../types";

const { Title, Text } = Typography;

type Notice = { tone: "success" | "warning" | "error"; text: string } | null;

function currentParams(rule: Rule, overrides: Record<string, string> = {}): Record<string, string> {
  const params: Record<string, string> = {};
  for (const param of rule.params) {
    params[param.key] = overrides[param.key] ?? param.value;
  }
  return params;
}

interface CardProps {
  rule: Rule;
  editing: Record<string, string>;
  onEdit: (key: string, value: string) => void;
  onCommit: (rule: Rule, overrides?: Record<string, string>, enabled?: boolean) => void;
  busy: boolean;
  onNotice: (notice: Notice) => void;
}

function RuleCard({ rule, editing, onEdit, onCommit, busy, onNotice }: CardProps) {
  return (
    <Card className="cg-rule-card">
      {/* 标题放在 body 里，和描述共用同一份 padding，避免 Card head 的额外留白与分隔线 */}
      <div className="cg-rule-card__head">
        <div className="cg-rule-card__heading">
          <span className="cg-rule-card__title">{rule.label}</span>
          <Text type="secondary" className="cg-rule-card__id">
            {rule.id}
          </Text>
        </div>
        <Switch
          checked={rule.enabled}
          disabled={busy}
          onChange={(checked) => onCommit(rule, undefined, checked)}
          aria-label={`启用 ${rule.label}`}
        />
      </div>
      <p className="cg-rule-card__desc">{rule.description}</p>

      {rule.params.length > 0 ? (
        <div className="cg-rule-card__params">
          {rule.params.map((param) => {
            const key = `${rule.id}:${param.key}`;
            const value = editing[key] ?? param.value;
            const invalid =
              param.required_placeholder !== null && !value.includes(param.required_placeholder);
            return (
              <label className="cg-field" key={param.key}>
                <span className="cg-field__label">
                  {param.label}
                  {param.required_placeholder ? (
                    <Text type="secondary" className="cg-field__hint">
                      须含 {param.required_placeholder}
                    </Text>
                  ) : null}
                </span>
                <Input
                  status={invalid ? "error" : undefined}
                  value={value}
                  spellCheck={false}
                  onChange={(event) => onEdit(key, event.target.value)}
                  onBlur={() => {
                    if (invalid) {
                      onNotice({
                        tone: "error",
                        text: `模板必须包含 ${param.required_placeholder}`,
                      });
                      return;
                    }
                    onCommit(rule, { [param.key]: value });
                  }}
                />
              </label>
            );
          })}
        </div>
      ) : (
        <Text type="secondary" className="cg-rule-card__noparam">
          该规则没有可调参数
        </Text>
      )}

      <div className="cg-rule-card__foot">
        <Text type="secondary">累计命中</Text>
        <span className="cg-rule-card__hits">{rule.hits}</span>
      </div>
    </Card>
  );
}

export function Rules() {
  const { data, isLoading, isError, error } = useStatus();
  const update = useRuleUpdate();
  const discard = useRuleDiscard();
  const [editing, setEditing] = useState<Record<string, string>>({});
  const [notice, setNotice] = useState<Notice>(null);

  if (isLoading) return <div className="cg-empty">正在读取规则…</div>;
  if (isError) return <div className="cg-empty cg-empty--error">读取规则失败：{String(error)}</div>;
  if (!data) return <div className="cg-empty">没有规则数据</div>;

  const busy = update.isPending || discard.isPending;

  const commit = (rule: Rule, overrides?: Record<string, string>, enabled = rule.enabled) => {
    const params = currentParams(rule, overrides);
    update.mutate(
      { id: rule.id, enabled, params },
      {
        onSuccess: (result) => {
          setNotice(
            result.persisted
              ? { tone: "success", text: "已生效并写入 config.toml" }
              : {
                  tone: "warning",
                  text: `已生效，但写入配置失败：${result.save_error ?? "未知原因"}`,
                },
          );
          setEditing((prev) => {
            const next = { ...prev };
            for (const key of Object.keys(next)) {
              if (key.startsWith(`${rule.id}:`)) delete next[key];
            }
            return next;
          });
        },
        onError: (mutationError) => setNotice({ tone: "error", text: String(mutationError) }),
      },
    );
  };

  return (
    <>
      <Title level={3} className="cg-page-title">
        兼容规则
      </Title>
      <div className="cg-subtitle">
        <Text type="secondary">
          开关与参数改完立即生效，并同步写回 config.toml（写前备份为
          config.toml.bak，注释与段落顺序保留）。
        </Text>
      </div>

      <Space className="cg-toolbar" wrap>
        <Tag color={data.rules_dirty ? "orange" : "success"}>
          {data.rules_dirty ? "有改动尚未写入" : "配置已同步"}
        </Tag>
        <span className="cg-toolbar__spacer" />
        <Button
          disabled={busy}
          onClick={() =>
            discard.mutate(undefined, {
              onSuccess: () => {
                setNotice({ tone: "success", text: "已从 config.toml 重新读取" });
                setEditing({});
              },
              onError: (mutationError) => setNotice({ tone: "error", text: String(mutationError) }),
            })
          }
        >
          从文件重读
        </Button>
      </Space>

      {notice ? (
        <Alert className="cg-alert" type={notice.tone} showIcon message={notice.text} />
      ) : null}

      <div className="cg-rule-list">
        {data.rules.map((rule) => (
          <RuleCard
            key={rule.id}
            rule={rule}
            editing={editing}
            busy={busy}
            onEdit={(key, value) => setEditing((prev) => ({ ...prev, [key]: value }))}
            onCommit={commit}
            onNotice={setNotice}
          />
        ))}
      </div>
    </>
  );
}
