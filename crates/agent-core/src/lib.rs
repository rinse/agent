//! `agent-core` — エージェントの純粋なコア。
//!
//! このクレートは副作用の具体実装を持たない。会話の状態（[`ConversationHistory`]）・
//! 基本型（メッセージ・ツール呼び出し）・ポート trait（[`ports`]）・
//! ループ駆動部（[`runner`]）を定義する。副作用は [`ports`] trait を通じて外部（アダプタ）へ
//! 委譲されるため、フェイク実装を差し込むだけでループ全体を決定的に検証できる。
//!
//! # モジュール構成
//!
//! - [`message`] … User / Assistant でコンテンツ型を分けたメッセージ
//! - [`tool`] … ツール使用要求（[`ToolCall`]）と実行結果（[`ToolResult`]）
//! - [`history`] … 不変に扱える会話履歴（[`ConversationHistory`]）
//! - [`turn`] … ツール実行ループの状態遷移（[`Turn`] / [`StopReason`]）
//! - [`ports`] … 副作用ポートの trait 定義（[`LanguageModel`] / [`Tool`] / [`ToolRegistry`]）
//! - [`runner`] … ツール実行ループ駆動部（[`TurnRunner`]）
//!
//! # serde 表現について
//!
//! 各型は `serde` の既定（externally-tagged）で直列化する。これは**内部の永続化・
//! ラウンドトリップ用途**には十分だが、実 LLM / MCP のワイヤ形式（例: Anthropic の
//! `{"type":"tool_use", ...}`）とは一致しない。ワイヤ表現が必要になったら、コア型を
//! 汚さずアダプタ側で変換するか、後続タスクで `#[serde(tag = "type", ...)]` 等の調整を
//! 入れる。

pub mod history;
pub mod message;
pub mod ports;
pub mod runner;
pub mod tool;
pub mod turn;

pub use history::ConversationHistory;
pub use message::{AssistantContent, AssistantMessage, Message, UserContent, UserMessage};
pub use ports::{
    EmptyToolRegistry, LanguageModel, ModelError, ModelResponse, Tool, ToolRegistry,
};
pub use runner::{RunError, TurnRunner};
pub use tool::{ToolCall, ToolOutcome, ToolResult, ToolUseId};
pub use turn::{StopReason, Turn, TurnError};
