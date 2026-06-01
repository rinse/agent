//! フェイク実装。ユニットテストでポートを差し替えるために使う。
//!
//! [`FakeLanguageModel`] と [`FakeToolRegistry`] / [`FakeTool`] を組み合わせれば、
//! 実 LLM や外部プロセスなしにループ全体を決定的に検証できる。

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use agent_core::{
    message::{AssistantContent, AssistantMessage, Message, UserContent},
    ports::{LanguageModel, ModelError, ModelResponse, Tool, ToolRegistry},
    tool::{ToolCall, ToolResult},
    turn::StopReason,
    ConversationHistory,
};
use async_trait::async_trait;

// ─────────────────────────────────────────────
// FakeLanguageModel
// ─────────────────────────────────────────────

/// スクリプト化されたレスポンスを返すフェイク LLM。
///
/// コンストラクタに渡した [`StopReason`] のキューを FIFO で返す。
/// キューが空になると [`ModelError`] を返す。
///
/// ## 使い方
///
/// ```rust,ignore
/// let model = FakeLanguageModel::scripted([
///     StopReason::ToolUse(vec![call]),
///     StopReason::EndTurn(AssistantMessage::text("done")),
/// ]);
/// ```
pub struct FakeLanguageModel {
    responses: Mutex<VecDeque<(AssistantMessage, StopReason)>>,
}

impl FakeLanguageModel {
    /// [`StopReason`] のリストからフェイクモデルを作る。
    ///
    /// - `EndTurn(msg)` → `ModelResponse.message = msg.clone()`
    /// - `ToolUse(calls)` → `ModelResponse.message` を呼び出し一覧から自動組み立て
    /// - その他 → 空の `AssistantMessage`
    pub fn scripted(stop_reasons: impl IntoIterator<Item = StopReason>) -> Self {
        let queue = stop_reasons
            .into_iter()
            .map(|stop| {
                let message = match &stop {
                    StopReason::EndTurn(msg) => msg.clone(),
                    StopReason::ToolUse(calls) => AssistantMessage {
                        content: calls
                            .iter()
                            .map(|c| AssistantContent::ToolUse(c.clone()))
                            .collect(),
                    },
                    _ => AssistantMessage::new(),
                };
                (message, stop)
            })
            .collect();
        Self {
            responses: Mutex::new(queue),
        }
    }
}

#[async_trait]
impl LanguageModel for FakeLanguageModel {
    async fn complete(&self, _history: &ConversationHistory) -> Result<ModelResponse, ModelError> {
        let (message, stop_reason) = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError::new("no more scripted responses"))?;
        Ok(ModelResponse {
            message,
            stop_reason,
        })
    }
}

// ─────────────────────────────────────────────
// EchoLanguageModel
// ─────────────────────────────────────────────

/// ユーザーの最新テキストをオウム返しするデモ用モデル。
///
/// 常に `StopReason::EndTurn` を返すため、ツール実行ループには入らない。
/// REPL の結合テストや CLI の動作確認に使う。
pub struct EchoLanguageModel;

#[async_trait]
impl LanguageModel for EchoLanguageModel {
    async fn complete(&self, history: &ConversationHistory) -> Result<ModelResponse, ModelError> {
        let user_text = history
            .messages()
            .iter()
            .rev()
            .find_map(|msg| match msg {
                Message::User(u) => u.content.iter().find_map(|c| match c {
                    UserContent::Text(t) => Some(t.as_str()),
                    _ => None,
                }),
                _ => None,
            })
            .unwrap_or("...");

        let message = AssistantMessage::text(format!("Echo: {user_text}"));
        Ok(ModelResponse {
            stop_reason: StopReason::EndTurn(message.clone()),
            message,
        })
    }
}

// ─────────────────────────────────────────────
// FakeTool
// ─────────────────────────────────────────────

/// 常に固定値を返すフェイクツール。
pub struct FakeTool {
    name: String,
    response: serde_json::Value,
}

impl FakeTool {
    pub fn new(name: impl Into<String>, response: serde_json::Value) -> Self {
        Self {
            name: name.into(),
            response,
        }
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        &self.name
    }

    async fn invoke(&self, call: &ToolCall) -> ToolResult {
        ToolResult::success(call.id.clone(), self.response.clone())
    }
}

// ─────────────────────────────────────────────
// FakeToolRegistry
// ─────────────────────────────────────────────

/// ツール名から [`FakeTool`] を解決するフェイクレジストリ。
///
/// ## 使い方
///
/// ```rust,ignore
/// let tools = FakeToolRegistry::new()
///     .with_tool(FakeTool::new("search", json!({"result": "ok"})));
/// ```
pub struct FakeToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl FakeToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// ツールを追加して `self` を返す（ビルダーパターン）。
    pub fn with_tool(mut self, tool: impl Tool + 'static) -> Self {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
        self
    }
}

impl Default for FakeToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry for FakeToolRegistry {
    fn resolve(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }
}

// ─────────────────────────────────────────────
// Tests（TurnRunner との統合テスト）
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        message::{AssistantMessage, Message, UserContent, UserMessage},
        tool::ToolUseId,
        turn::StopReason,
        ConversationHistory, TurnRunner,
    };
    use serde_json::json;

    fn make_call(id: &str, name: &str) -> ToolCall {
        ToolCall::new(ToolUseId::new(id), name, json!({}))
    }

    #[tokio::test]
    async fn runner_end_turn_immediately() {
        let model =
            FakeLanguageModel::scripted([StopReason::EndTurn(AssistantMessage::text("hello"))]);
        let runner = TurnRunner::new(model, FakeToolRegistry::new());
        let mut history = ConversationHistory::new().with(UserMessage::text("hi"));

        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("hello"));
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn runner_tool_use_then_end_turn() {
        let call = make_call("c1", "search");
        let model = FakeLanguageModel::scripted([
            StopReason::ToolUse(vec![call.clone()]),
            StopReason::EndTurn(AssistantMessage::text("found it")),
        ]);
        let tools = FakeToolRegistry::new()
            .with_tool(FakeTool::new("search", json!({"result": "Rust is great"})));
        let runner = TurnRunner::new(model, tools);

        let mut history = ConversationHistory::new().with(UserMessage::text("search for rust"));
        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("found it"));
        // user + assistant(tool_use) + user(tool_result) + assistant(end_turn) = 4
        assert_eq!(history.len(), 4);

        // ツール結果が成功値を持つか確認。
        if let Message::User(u) = &history.messages()[2] {
            if let UserContent::ToolResult(r) = &u.content[0] {
                assert!(!r.is_error());
            } else {
                panic!("expected ToolResult");
            }
        } else {
            panic!("expected User message at index 2");
        }
    }

    #[tokio::test]
    async fn runner_unknown_tool_yields_error_result() {
        let call = make_call("c1", "unknown");
        let model = FakeLanguageModel::scripted([
            StopReason::ToolUse(vec![call]),
            StopReason::EndTurn(AssistantMessage::text("ok")),
        ]);
        let runner = TurnRunner::new(model, FakeToolRegistry::new()); // ツールなし

        let mut history = ConversationHistory::new().with(UserMessage::text("do it"));
        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("ok"));
        if let Message::User(u) = &history.messages()[2] {
            if let UserContent::ToolResult(r) = &u.content[0] {
                assert!(r.is_error());
            } else {
                panic!("expected ToolResult");
            }
        } else {
            panic!("expected User message at index 2");
        }
    }

    #[tokio::test]
    async fn runner_multiple_tools_in_one_round() {
        let c1 = make_call("c1", "search");
        let c2 = make_call("c2", "fetch");
        let model = FakeLanguageModel::scripted([
            StopReason::ToolUse(vec![c1, c2]),
            StopReason::EndTurn(AssistantMessage::text("all done")),
        ]);
        let tools = FakeToolRegistry::new()
            .with_tool(FakeTool::new("search", json!("result")))
            .with_tool(FakeTool::new("fetch", json!("page")));
        let runner = TurnRunner::new(model, tools);

        let mut history = ConversationHistory::new().with(UserMessage::text("go"));
        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("all done"));
        // user + asst(tu) + user(r1,r2) + asst(end) = 4
        assert_eq!(history.len(), 4);

        // user(tool_result) に 2 件の結果が入っているか。
        if let Message::User(u) = &history.messages()[2] {
            assert_eq!(u.content.len(), 2);
        } else {
            panic!("expected User message at index 2");
        }
    }

    #[tokio::test]
    async fn fake_tool_returns_scripted_value() {
        let tool = FakeTool::new("greet", json!("hello world"));
        let call = ToolCall::new(ToolUseId::new("c1"), "greet", json!({}));
        let result = tool.invoke(&call).await;
        assert!(!result.is_error());
        assert_eq!(
            result.outcome,
            agent_core::tool::ToolOutcome::Success(json!("hello world"))
        );
    }
}
