//! ポート定義: 副作用を持つ外部コンポーネントへのインターフェース。
//!
//! コアのループ駆動部が必要とする副作用（LLM 呼び出し・ツール実行）を trait として抽象化し、
//! 実装（アダプタ）を差し替え可能にする。テストではフェイクを差し込むだけでよい。
//!
//! ## 循環依存の回避
//!
//! 本来「ポート」は独立クレートに分けるのが理想だが、`ports` クレートが `agent-core` の型
//! （[`ConversationHistory`] 等）を参照し、さらに `agent-core` の runner が `ports` の trait を
//! 使うと循環依存が生じる。そのため T3 では trait 定義を `agent-core` 内に直接置き、
//! `ports` クレートはそれを再エクスポートする薄いラッパとする。
//!
//! ## `async fn` の object-safe 化
//!
//! `async fn` をトレイトに持たせると dyn 非互換になる（各実装が異なる Future 型を返すため）。
//! [`async_trait`] を使うことで `async fn` を `Pin<Box<dyn Future>>` に脱糖し、object-safe にする。
//! これにより `Box<dyn Tool>` / `Option<&dyn Tool>` が使える。

use std::fmt;

use async_trait::async_trait;

use crate::{
    message::AssistantMessage,
    tool::{ToolCall, ToolResult},
    turn::StopReason,
    ConversationHistory,
};

// ─────────────────────────────────────────────
// ModelError
// ─────────────────────────────────────────────

/// LLM 呼び出し時に発生するエラー。
///
/// 内部フィールドは `pub(crate)` に限定し、外部からは [`ModelError::new`] / `Display` / `Error`
/// 経由のみでアクセスさせる。将来エラーコードやコンテキスト情報を追加しても破壊的変更にならない。
#[derive(Debug)]
pub struct ModelError(pub(crate) String);

impl ModelError {
    pub fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "model error: {}", self.0)
    }
}

impl std::error::Error for ModelError {}

// ─────────────────────────────────────────────
// ModelResponse
// ─────────────────────────────────────────────

/// モデルが 1 回の [`LanguageModel::complete`] 呼び出しで返す応答。
///
/// `message` と `stop_reason` を分けているのは、`ToolUse` 時にも生成されたアシスタント
/// メッセージ全体（テキスト＋ツール使用要求）を会話履歴へ追記するため。
/// `EndTurn` 時は `message` が [`StopReason::EndTurn`] 内の値と等価になる。
///
/// # 設計上の既知課題
///
/// `StopReason::EndTurn(AssistantMessage)` が既にメッセージを保持しているため、
/// `EndTurn` 時に `message` フィールドとの間で不一致が生じるリスクがある。
/// 将来的には `StopReason::EndTurn` からメッセージを取り除き、常に `message` フィールドを
/// 使う方向への統一を検討すること。
pub struct ModelResponse {
    /// アシスタントが生成したメッセージ全体。
    ///
    /// ループ駆動部はこれを履歴へ追記してから `stop_reason` を処理する。
    pub message: AssistantMessage,
    /// 生成が止まった理由。ループの継続/終了を決める値。
    pub stop_reason: StopReason,
}

// ─────────────────────────────────────────────
// LanguageModel
// ─────────────────────────────────────────────

/// LLM への問い合わせポート。
///
/// 会話履歴を受け取り、モデルの応答（メッセージ＋停止理由）を返す。
/// 実装は Anthropic API クライアント、ローカル LLM、テスト用フェイクなど様々を差し込める。
#[async_trait]
pub trait LanguageModel: Send + Sync {
    async fn complete(
        &self,
        history: &ConversationHistory,
    ) -> Result<ModelResponse, ModelError>;
}

// ─────────────────────────────────────────────
// Tool / ToolRegistry
// ─────────────────────────────────────────────

/// ツール実行ポート（単一ツール）。
///
/// `invoke` は呼び出し側が渡した [`ToolCall`] を受け取り、[`ToolResult`] を返す。
/// ツールの失敗は `Result::Err` でなく [`ToolResult`] の中の [`crate::tool::ToolOutcome::Error`]
/// として返すことでループを止めない（設計方針に従う）。
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    async fn invoke(&self, call: &ToolCall) -> ToolResult;
}

/// ツール名からツールを解決するレジストリ。
///
/// `resolve` が `Option<&dyn Tool>` を返すため、[`Tool`] trait が object-safe でなければならない。
/// `#[async_trait]` により `Tool` は object-safe になっている。
pub trait ToolRegistry: Send + Sync {
    fn resolve(&self, name: &str) -> Option<&dyn Tool>;
}

/// ツールを持たない空のレジストリ。
///
/// ツールを使わないシナリオのテストや、ToolRegistry を省略したい場面で使う。
pub struct EmptyToolRegistry;

impl ToolRegistry for EmptyToolRegistry {
    fn resolve(&self, _name: &str) -> Option<&dyn Tool> {
        None
    }
}
