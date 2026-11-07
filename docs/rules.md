# 规则开发指南

这个项目的核心抽象只有一条：`RewriteRule`。所有兼容性问题都应该被表达成一条规则，
而不是散落在转发逻辑里。

## 契约

```rust
pub trait RewriteRule: Send + Sync {
    /// 稳定标识，同时是 config.toml 里的键名。改了会导致用户配置失效。
    fn id(&self) -> &'static str;

    /// 面向人的说明，出现在 /_gateway/stats 里。
    fn label(&self) -> &'static str;

    /// 原地改写请求体，返回本次命中次数。
    /// 返回 0 表示没有改动，注册表不会累加统计。
    fn apply(&self, body: &mut Value) -> usize;
}
```

四条硬约束：

1. **幂等**。同一 body 连续过两遍结果一致。上游可能重试，规则重复执行不能越改越坏。
2. **局部**。只碰自己认识的结构。找不到目标时返回 0，不要"顺手规范化"其他字段。
3. **无副作用**。不读配置、不发请求、不写文件。需要开关就让 `Registry` 在外面判断。
4. **可降级**。改写失败不能让整个请求失败，最差也要保持原样。

## 现有规则

| id | 处理 | 对应上游报错 | 文件 |
| --- | --- | --- | --- |
| `thinking_disable` | 删除顶层 `thinking` 字段（仅当 `type == "disabled"`） | `400 thinking.type=disabled cannot be translated` | `src/rewrite/thinking_disable.rs` |
| `tool_reference` | `tool_result.content` 里的 `tool_reference` 块降级为 text | `500 Invalid content item type: tool_reference` | `src/rewrite/tool_reference.rs` |

## 从零加一条规则

### 1. 先确认上游到底卡在哪个字段

不要凭猜。抓一个真实失败的请求体，或者从上游报错文案里定位字段名。
报错文案通常长这样：

```
Invalid content item type: tool_reference. Expected str or dict with 'type' field
(text, image_url, image, document, tool_use, tool_result, thinking).
```

括号里那份白名单就是上游认识的类型，不在里面的都是嫌疑对象。

### 2. 写规则文件

以"把某个布尔字段转成字符串"这种假设需求为例：

```rust
use super::RewriteRule;
use serde_json::Value;

pub struct MyRule;

impl RewriteRule for MyRule {
    fn id(&self) -> &'static str {
        "my_rule"
    }

    fn label(&self) -> &'static str {
        "把 foo.enabled 布尔值转成字符串"
    }

    fn apply(&self, body: &mut Value) -> usize {
        let Some(foo) = body.get_mut("foo").and_then(Value::as_object_mut) else {
            return 0;
        };
        let Some(enabled) = foo.get("enabled").and_then(Value::as_bool) else {
            return 0;
        };
        // 已经是目标形态就不动，保证幂等
        foo.insert("enabled".into(), Value::String(enabled.to_string()));
        1
    }
}
```

注意这段代码的几个习惯：

- 用 `let Some(..) = ... else { return 0 }` 逐层收敛，不写嵌套 if
- 先判断"当前是不是目标形态"，保证二次执行不会重复改
- 返回的 `1` 是"改了一处"，不是"改了一个字段组"，统计口径要一致

### 3. 注册

`crates/gateway-core/src/rewrite/mod.rs`：

```rust
pub fn with_default_rules() -> Self {
    let mut registry = Self::new();
    registry.register(Box::new(thinking_disable::ThinkingDisable));
    registry.register(Box::new(tool_reference::ToolReferenceDowngrade));
    registry.register(Box::new(my_rule::MyRule));   // 加这一行
    registry
}
```

顺序有意义：注册顺序就是执行顺序。如果有规则依赖前一条的结果，在这里排好并写注释说明。

别忘了文件顶部 `pub mod my_rule;`。

### 4. 配置开关

`config.toml`：

```toml
[rules.my_rule]
enabled = true
```

不写这一段也视为启用（`Config::rule_enabled` 的默认是 true）。显式写出来是为了让人一眼看到有哪些规则。

需要临时关掉某条规则定位问题时，把它改成 false 重启即可，不用改代码。

### 5. 测试

`crates/gateway-core/tests/rewrite_test.rs` 至少三个用例：

```rust
#[test]
fn my_rule_hits_when_expected() {
    let mut body = json!({ "foo": { "enabled": true } });
    assert_eq!(MyRule.apply(&mut body), 1);
    assert_eq!(body["foo"]["enabled"], "true");
}

#[test]
fn my_rule_ignores_other_shapes() {
    let mut body = json!({ "foo": { "enabled": "already-string" } });
    assert_eq!(MyRule.apply(&mut body), 0);
}

#[test]
fn my_rule_is_idempotent() {
    let mut body = json!({ "foo": { "enabled": true } });
    MyRule.apply(&mut body);
    let once = body.clone();
    assert_eq!(MyRule.apply(&mut body), 0);
    assert_eq!(body, once);
}
```

第三条容易被跳过，但它是最容易在重试场景里炸的地方。

### 6. 更新文档

- 本文档上面的规则清单表加一行
- `docs/troubleshooting.md` 写清这条规则对应什么报错现象、怎么判断它没生效

## 反模式

**不要写"万能清理"规则。** 比如"删掉所有不认识的顶层字段"。
上游可能新增合法字段，一刀切会把正常请求改坏，而且现象是"模型变笨了"，
根本联想不到是网关改的。

**不要在规则里读环境变量或配置。** 开关的判断权在 `Registry`，规则只做纯转换。
否则同一条规则在不同调用路径上行为不一致。

**不要改响应。** 这个项目的边界是请求方向。响应改写会牵扯流式解析、SSE 事件边界、
token 计费口径，复杂度是另一个量级。

**不要为了省事改 `forward.rs`。** 如果发现"只在某种请求上才改写"，正确做法是让规则自己判断，
而不是在转发层加条件分支。

**不要在规则里 panic。** 用 `?`、`let else`、`unwrap_or` 处理所有 `Option`。
一个畸形请求不应该让整个网关退出。

## 什么时候该删掉一条规则

上游网关升级后，可能已经原生支持了某个字段。判断依据：

- `/_gateway/stats` 里这条规则的 `hits` 一直在涨，但上游其实不再报错了 → 规则是多余的
- 关掉这条规则的开关，观察一天，请求正常 → 可以删

删的时候连带清理：注册行、配置段落、测试用例、两份文档里的条目。
