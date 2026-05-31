//! ツール実行ループの駆動部（`TurnRunner`）。
//!
//! `TurnRunner` は 1 ターン分のエージェントループを [`Turn`] 状態機械で管理し、
//! 実際の副作用は [`LanguageModel`] / [`ToolRegistry`] ポートへ委譲する。
//! ポートにフェイクを差し込むだけでループ全体を決定的に検証できる。
//!
//! ## ループの流れ
//!
//! ```text
//! AwaitingModel
//!   │  model.complete(history) を呼ぶ
//!   │  → アシスタントメッセージを history に追記
//!   ├─(EndTurn)──→ Completed → 最終メッセージを返す
//!   └─(ToolUse)──→ ExecutingTools
//!                    │  各ツールを逐次実行
//!                    │  → ツール結果を history に追記
//!                    └──→ AwaitingModel（再びモデルへ問い合わせ）
//! ```

use crate::{
    message::{AssistantMessage, UserMessage},
    ports::{LanguageModel, ModelError, ToolRegistry},
    tool::{ToolCall, ToolResult},
    turn::{Turn, TurnError},
    ConversationHistory,
};

// ─────────────────────────────────────────────
// RunError
// ─────────────────────────────────────────────

/// ターン実行中に発生するエラー。
#[derive(Debug)]
pub enum RunError {
    /// モデル呼び出しが失敗した。
    Model(ModelError),
    /// ターン状態機械の遷移に問題があった（ループ駆動部のバグ、またはモデルの不正応答）。
    Turn(TurnError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Model(e) => write!(f, "model error: {e}"),
            RunError::Turn(e) => write!(f, "turn error: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

impl From<ModelError> for RunError {
    fn from(e: ModelError) -> Self {
        RunError::Model(e)
    }
}

impl From<TurnError> for RunError {
    fn from(e: TurnError) -> Self {
        RunError::Turn(e)
    }
}

// ─────────────────────────────────────────────
// TurnRunner
// ─────────────────────────────────────────────

/// 1 ターン（ツール実行ループ）を駆動する実行器。
///
/// - `M`: [`LanguageModel`] 実装。
/// - `R`: [`ToolRegistry`] 実装。
///
/// 両ポートを Dependency Injection で受け取るため、テストでは差し替えが容易。
pub struct TurnRunner<M, R> {
    model: M,
    tools: R,
}

impl<M: LanguageModel, R: ToolRegistry> TurnRunner<M, R> {
    /// 新しい `TurnRunner` を組み立てる。
    pub fn new(model: M, tools: R) -> Self {
        Self { model, tools }
    }

    /// `history` を文脈として 1 ターン分のツール実行ループを回す。
    ///
    /// ループ中に生成されたアシスタントメッセージとツール結果は `history` に追記される。
    /// エラー時も追記済みの分はそのまま残る（デバッグログとして利用可能）。
    pub async fn run(&self, history: &mut ConversationHistory) -> Result<AssistantMessage, RunError> {
        let mut turn = Turn::AwaitingModel;

        loop {
            turn = match turn {
                Turn::AwaitingModel => {
                    let response = self.model.complete(history).await?;
                    // ツール使用時もアシスタントメッセージ全体（テキスト＋ツール要求）を履歴へ追記する。
                    history.push(response.message.clone());
                    Turn::AwaitingModel.on_model_response(response.stop_reason)?
                }

                Turn::ExecutingTools(calls) => {
                    // ツールを逐次実行する（並行実行は後続タスクで検討）。
                    let results = self.execute_calls(&calls).await;
                    history.push(UserMessage::tool_results(results));
                    Turn::ExecutingTools(calls).on_tools_completed()?
                }

                Turn::Completed(msg) => return Ok(msg),
            };
        }
    }

    /// `calls` に含まれる各ツール呼び出しを逐次実行し、結果を収集する。
    ///
    /// レジストリに存在しないツール名は [`ToolResult::error`] として記録し、ループを継続する。
    /// ツール失敗はループを止めず、モデルへ結果を戻して回復を促す（設計方針に従う）。
    async fn execute_calls(&self, calls: &[ToolCall]) -> Vec<ToolResult> {
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            let result = match self.tools.resolve(&call.name) {
                Some(tool) => tool.invoke(call).await,
                None => ToolResult::error(
                    call.id.clone(),
                    format!("unknown tool: {}", call.name),
                ),
            };
            results.push(result);
        }
        results
    }
}

// ─────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        message::{AssistantContent, AssistantMessage, Message, UserContent, UserMessage},
        ports::{EmptyToolRegistry, ModelError, ModelResponse},
        tool::{ToolCall, ToolUseId},
        turn::StopReason,
        ConversationHistory,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use std::{collections::VecDeque, sync::Mutex};

    // ── インラインフェイク ──────────────────────

    struct FakeModel(Mutex<VecDeque<(AssistantMessage, StopReason)>>);

    impl FakeModel {
        fn scripted(stop_reasons: impl IntoIterator<Item = StopReason>) -> Self {
            let queue = stop_reasons
                .into_iter()
                .map(|stop| {
                    let msg = match &stop {
                        StopReason::EndTurn(m) => m.clone(),
                        StopReason::ToolUse(calls) => AssistantMessage {
                            content: calls
                                .iter()
                                .map(|c| AssistantContent::ToolUse(c.clone()))
                                .collect(),
                        },
                        _ => AssistantMessage::new(),
                    };
                    (msg, stop)
                })
                .collect();
            Self(Mutex::new(queue))
        }
    }

    #[async_trait]
    impl LanguageModel for FakeModel {
        async fn complete(&self, _: &ConversationHistory) -> Result<ModelResponse, ModelError> {
            let (message, stop_reason) = self
                .0
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

    // ── ヘルパ ──────────────────────────────────

    fn make_call(id: &str, name: &str) -> ToolCall {
        ToolCall::new(ToolUseId::new(id), name, json!({}))
    }

    // ── テスト ──────────────────────────────────

    #[tokio::test]
    async fn end_turn_completes_and_appends_to_history() {
        let model =
            FakeModel::scripted([StopReason::EndTurn(AssistantMessage::text("done"))]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("hi"));

        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("done"));
        // user + assistant(end_turn)
        assert_eq!(history.len(), 2);
        assert!(matches!(history.last(), Some(Message::Assistant(_))));
    }

    #[tokio::test]
    async fn tool_use_then_end_turn_builds_correct_history() {
        let call = make_call("c1", "search");
        let model = FakeModel::scripted([
            StopReason::ToolUse(vec![call.clone()]),
            StopReason::EndTurn(AssistantMessage::text("found")),
        ]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("search?"));

        let result = runner.run(&mut history).await.unwrap();

        assert_eq!(result, AssistantMessage::text("found"));
        // user + assistant(tool_use) + user(tool_result) + assistant(end_turn) = 4
        assert_eq!(history.len(), 4);
    }

    #[tokio::test]
    async fn unknown_tool_produces_error_result_and_loop_continues() {
        let call = make_call("c1", "nonexistent");
        let model = FakeModel::scripted([
            StopReason::ToolUse(vec![call]),
            StopReason::EndTurn(AssistantMessage::text("recovered")),
        ]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("do it"));

        let result = runner.run(&mut history).await.unwrap();
        assert_eq!(result, AssistantMessage::text("recovered"));

        // ツール結果が error になっているか確認。
        let tool_result_msg = &history.messages()[2];
        if let Message::User(u) = tool_result_msg {
            if let UserContent::ToolResult(r) = &u.content[0] {
                assert!(r.is_error());
                assert!(matches!(&r.outcome, crate::tool::ToolOutcome::Error(msg) if msg.contains("nonexistent")));
            } else {
                panic!("expected ToolResult content");
            }
        } else {
            panic!("expected User message at index 2");
        }
    }

    #[tokio::test]
    async fn two_tool_rounds_complete_correctly() {
        let c1 = make_call("c1", "search");
        let c2 = make_call("c2", "fetch");
        let model = FakeModel::scripted([
            StopReason::ToolUse(vec![c1]),
            StopReason::ToolUse(vec![c2]),
            StopReason::EndTurn(AssistantMessage::text("all done")),
        ]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("go"));

        let result = runner.run(&mut history).await.unwrap();
        assert_eq!(result, AssistantMessage::text("all done"));
        // user + asst(tu1) + user(r1) + asst(tu2) + user(r2) + asst(end) = 6
        assert_eq!(history.len(), 6);
    }

    #[tokio::test]
    async fn max_tokens_returns_run_error_turn() {
        let model = FakeModel::scripted([StopReason::MaxTokens]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("hi"));

        let err = runner.run(&mut history).await.unwrap_err();
        assert!(matches!(
            err,
            RunError::Turn(TurnError::MaxTokensReached)
        ));
    }

    #[tokio::test]
    async fn cancelled_returns_run_error_turn() {
        let model = FakeModel::scripted([StopReason::Cancelled]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("hi"));

        let err = runner.run(&mut history).await.unwrap_err();
        assert!(matches!(err, RunError::Turn(TurnError::Cancelled)));
    }

    #[tokio::test]
    async fn empty_scripted_model_returns_model_error() {
        let model = FakeModel::scripted([]); // 空キュー
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("hi"));

        let err = runner.run(&mut history).await.unwrap_err();
        assert!(matches!(err, RunError::Model(_)));
    }

    #[tokio::test]
    async fn tool_use_stop_reason_assistant_message_in_history() {
        let call = make_call("c1", "calc");
        let model = FakeModel::scripted([
            StopReason::ToolUse(vec![call.clone()]),
            StopReason::EndTurn(AssistantMessage::text("done")),
        ]);
        let runner = TurnRunner::new(model, EmptyToolRegistry);
        let mut history = ConversationHistory::new().with(UserMessage::text("calc"));

        runner.run(&mut history).await.unwrap();

        // history[1] は ToolUse を含むアシスタントメッセージのはず。
        if let Message::Assistant(a) = &history.messages()[1] {
            assert!(a.requests_tools());
            let calls: Vec<_> = a.tool_calls().collect();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "calc");
        } else {
            panic!("expected Assistant at index 1");
        }
    }
}
