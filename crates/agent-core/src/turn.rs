//! ツール実行ループの状態遷移。
//!
//! エージェントの 1 ターンは、モデルへ問い合わせてツールを実行し、
//! 最終応答が確定するまで繰り返す内側のループ。その状態は [`Turn`] で表現する。
//!
//! ループを止める条件は「モデルがなぜ生成を止めたか」、すなわち [`StopReason`] によって決まる。
//! 状態遷移は [`Turn::on_model_response`] と [`Turn::on_tools_completed`] の 2 種類の
//! 純粋関数として表現しており、副作用は一切持たない。
//!
//! ```text
//! AwaitingModel ─(EndTurn)──────────────────→ Completed
//!      │
//!      └─(ToolUse)──→ ExecutingTools ─(完了)──→ AwaitingModel
//! ```

use serde::{Deserialize, Serialize};

use crate::message::AssistantMessage;
use crate::tool::ToolCall;

/// モデルが生成を停止した理由。ツール実行ループの継続/終了を駆動する値。
///
/// ループ駆動部はこの値だけを見て「続けるか / 終わるか / 打ち切るか」を判断する。
/// `MaxTokens` / `Cancelled` は回復不能な終端であり、[`TurnError`] へ変換される。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StopReason {
    /// 通常完了。アシスタントのメッセージをターン完了として返す。
    EndTurn(AssistantMessage),
    /// ツール実行要求。指定されたツールを実行して結果を履歴へ戻し、ループを継続する。
    ToolUse(Vec<ToolCall>),
    /// トークン上限到達（回復不能）。
    MaxTokens,
    /// 外部からキャンセルされた。
    Cancelled,
}

/// 1 ターン（ツール実行ループ）の状態。
///
/// 各状態の意味:
/// - `AwaitingModel` : モデルへ問い合わせ待ち（ループの開始点・ツール実行後の再開点）
/// - `ExecutingTools`: ツールを実行中（実行すべき呼び出し一覧を保持）
/// - `Completed`     : ターン完了（最終的なアシスタントメッセージが確定）
///
/// 状態遷移は [`Turn::on_model_response`] / [`Turn::on_tools_completed`] の 2 関数に集約され、
/// ありえない遷移はコンパイル時ではなく実行時（`Err` 値）として検出される。
/// ループ駆動部がこれらを呼ぶ順序を守る限り `Err` は発生しない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Turn {
    /// モデルへ問い合わせ中。応答待ち状態。
    AwaitingModel,
    /// ツールを実行中。実行すべき呼び出しの一覧を保持する。
    ExecutingTools(Vec<ToolCall>),
    /// ターン完了。最終的なアシスタントメッセージが確定した。
    Completed(AssistantMessage),
}

impl Turn {
    /// `AwaitingModel` 状態でモデル応答を受け取り、次の状態へ遷移する。
    ///
    /// - `EndTurn`  → `Completed`
    /// - `ToolUse`  → `ExecutingTools`
    /// - `MaxTokens` / `Cancelled` → `Err`（回復不能）
    ///
    /// `AwaitingModel` 以外の状態から呼んだ場合は `Err(TurnError::InvalidTransition)`。
    pub fn on_model_response(self, reason: StopReason) -> Result<Turn, TurnError> {
        match self {
            Turn::AwaitingModel => match reason {
                StopReason::EndTurn(msg) => Ok(Turn::Completed(msg)),
                StopReason::ToolUse(calls) if calls.is_empty() => {
                    Err(TurnError::EmptyToolUse)
                }
                StopReason::ToolUse(calls) => Ok(Turn::ExecutingTools(calls)),
                StopReason::MaxTokens => Err(TurnError::MaxTokensReached),
                StopReason::Cancelled => Err(TurnError::Cancelled),
            },
            _ => Err(TurnError::InvalidTransition),
        }
    }

    /// `ExecutingTools` 状態でツール実行が完了したことを受け取り、`AwaitingModel` へ戻る。
    ///
    /// ツール結果を [`crate::ConversationHistory`] へ追加するのはループ駆動部の責務であり、
    /// この純粋遷移関数はループ制御のみを担う。
    ///
    /// `&self` を取るのは遷移に呼び出し一覧（`Vec<ToolCall>`）が不要なため。所有権を消費せずに
    /// 状態チェックと次状態の生成ができる。
    ///
    /// `ExecutingTools` 以外の状態から呼んだ場合は `Err(TurnError::InvalidTransition)`。
    pub fn on_tools_completed(&self) -> Result<Turn, TurnError> {
        match self {
            Turn::ExecutingTools(_) => Ok(Turn::AwaitingModel),
            _ => Err(TurnError::InvalidTransition),
        }
    }

    /// ターンが完了状態（`Completed`）かどうか。
    pub fn is_completed(&self) -> bool {
        matches!(self, Turn::Completed(_))
    }

    /// 完了している場合にアシスタントメッセージを所有権ごと取り出す。
    pub fn into_completed(self) -> Option<AssistantMessage> {
        match self {
            Turn::Completed(msg) => Some(msg),
            _ => None,
        }
    }

    /// `ExecutingTools` 状態で実行すべきツール呼び出し一覧を借用で取り出す。
    ///
    /// 他の状態では `None` を返す。
    pub fn pending_calls(&self) -> Option<&[ToolCall]> {
        match self {
            Turn::ExecutingTools(calls) => Some(calls),
            _ => None,
        }
    }
}

/// ターン状態遷移で発生するエラー。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TurnError {
    /// ありえない状態遷移（ループ駆動部のバグ）。
    InvalidTransition,
    /// トークン上限到達。
    MaxTokensReached,
    /// 外部キャンセル。
    Cancelled,
    /// ToolUse に呼び出しが 1 件もない（モデルの不正応答）。
    EmptyToolUse,
    /// ツール実行ラウンドが上限に達した（モデルがツール要求を返し続けた）。
    MaxRoundsExceeded,
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::InvalidTransition => write!(f, "invalid state transition"),
            TurnError::MaxTokensReached => write!(f, "max tokens reached"),
            TurnError::Cancelled => write!(f, "cancelled"),
            TurnError::EmptyToolUse => write!(f, "ToolUse stop reason contained no tool calls"),
            TurnError::MaxRoundsExceeded => write!(f, "max rounds exceeded"),
        }
    }
}

impl std::error::Error for TurnError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::AssistantMessage;
    use crate::tool::{ToolCall, ToolUseId};
    use serde_json::json;

    fn make_call(id: &str) -> ToolCall {
        ToolCall::new(ToolUseId::new(id), "search", json!({}))
    }

    #[test]
    fn awaiting_model_end_turn_completes() {
        let msg = AssistantMessage::text("done");
        let next = Turn::AwaitingModel
            .on_model_response(StopReason::EndTurn(msg.clone()))
            .unwrap();
        assert_eq!(next, Turn::Completed(msg));
        assert!(next.is_completed());
    }

    #[test]
    fn awaiting_model_tool_use_moves_to_executing() {
        let calls = vec![make_call("c1")];
        let next = Turn::AwaitingModel
            .on_model_response(StopReason::ToolUse(calls.clone()))
            .unwrap();
        assert_eq!(next, Turn::ExecutingTools(calls));
        assert!(!next.is_completed());
    }

    #[test]
    fn awaiting_model_max_tokens_is_error() {
        let result = Turn::AwaitingModel.on_model_response(StopReason::MaxTokens);
        assert_eq!(result, Err(TurnError::MaxTokensReached));
    }

    #[test]
    fn awaiting_model_cancelled_is_error() {
        let result = Turn::AwaitingModel.on_model_response(StopReason::Cancelled);
        assert_eq!(result, Err(TurnError::Cancelled));
    }

    #[test]
    fn executing_tools_completed_returns_awaiting_model() {
        let next = Turn::ExecutingTools(vec![make_call("c1")])
            .on_tools_completed()
            .unwrap();
        assert_eq!(next, Turn::AwaitingModel);
    }

    #[test]
    fn executing_tools_on_model_response_is_invalid() {
        let result = Turn::ExecutingTools(vec![make_call("c1")])
            .on_model_response(StopReason::EndTurn(AssistantMessage::text("x")));
        assert_eq!(result, Err(TurnError::InvalidTransition));
    }

    #[test]
    fn completed_on_model_response_is_invalid() {
        let result = Turn::Completed(AssistantMessage::text("done"))
            .on_model_response(StopReason::EndTurn(AssistantMessage::text("x")));
        assert_eq!(result, Err(TurnError::InvalidTransition));
    }

    #[test]
    fn awaiting_model_on_tools_completed_is_invalid() {
        let result = Turn::AwaitingModel.on_tools_completed();
        assert_eq!(result, Err(TurnError::InvalidTransition));
    }

    #[test]
    fn pending_calls_only_in_executing_state() {
        let calls = vec![make_call("c1"), make_call("c2")];
        assert_eq!(
            Turn::ExecutingTools(calls.clone()).pending_calls(),
            Some(calls.as_slice())
        );
        assert_eq!(Turn::AwaitingModel.pending_calls(), None);
        assert_eq!(
            Turn::Completed(AssistantMessage::text("done")).pending_calls(),
            None
        );
    }

    #[test]
    fn into_completed_extracts_message() {
        let msg = AssistantMessage::text("result");
        assert_eq!(
            Turn::Completed(msg.clone()).into_completed(),
            Some(msg)
        );
        assert_eq!(Turn::AwaitingModel.into_completed(), None);
    }

    #[test]
    fn full_turn_cycle_with_one_tool_call() {
        // AwaitingModel → ExecutingTools → AwaitingModel → Completed
        let turn = Turn::AwaitingModel
            .on_model_response(StopReason::ToolUse(vec![make_call("c1")]))
            .unwrap();
        let turn = turn.on_tools_completed().unwrap();
        assert_eq!(turn, Turn::AwaitingModel);

        let final_msg = AssistantMessage::text("finished");
        let turn = turn
            .on_model_response(StopReason::EndTurn(final_msg.clone()))
            .unwrap();
        assert_eq!(turn.into_completed(), Some(final_msg));
    }

    #[test]
    fn full_turn_cycle_with_two_tool_rounds() {
        // AwaitingModel → ExecutingTools → AwaitingModel → ExecutingTools → AwaitingModel → Completed
        let turn = Turn::AwaitingModel
            .on_model_response(StopReason::ToolUse(vec![make_call("c1")]))
            .unwrap()
            .on_tools_completed()
            .unwrap()
            .on_model_response(StopReason::ToolUse(vec![make_call("c2")]))
            .unwrap()
            .on_tools_completed()
            .unwrap()
            .on_model_response(StopReason::EndTurn(AssistantMessage::text("done")))
            .unwrap();
        assert!(turn.is_completed());
    }

    #[test]
    fn empty_tool_use_is_error() {
        let result = Turn::AwaitingModel.on_model_response(StopReason::ToolUse(vec![]));
        assert_eq!(result, Err(TurnError::EmptyToolUse));
    }

    #[test]
    fn turn_serializes_and_deserializes() {
        let turn = Turn::ExecutingTools(vec![make_call("c1")]);
        let encoded = serde_json::to_string(&turn).unwrap();
        let decoded: Turn = serde_json::from_str(&encoded).unwrap();
        assert_eq!(turn, decoded);
    }

    #[test]
    fn stop_reason_serializes_and_deserializes() {
        let msg = AssistantMessage::text("hello");
        let reason = StopReason::EndTurn(msg);
        let encoded = serde_json::to_string(&reason).unwrap();
        let decoded: StopReason = serde_json::from_str(&encoded).unwrap();
        assert_eq!(reason, decoded);
    }
}
