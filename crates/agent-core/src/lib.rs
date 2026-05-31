//! `agent-core` — エージェントの純粋なコア。
//!
//! このクレートは副作用を持たない。会話の状態（[`ConversationHistory`]）と、その上に載る
//! 基本型（メッセージ・ツール呼び出し）を定義する。LLM 呼び出しやツール実行といった副作用は
//! ポート（trait）越しに別レイヤへ追い出すため、ここで定義する型はすべてテストで決定的に検証できる。
//!
//! # モジュール構成
//!
//! - [`message`] … User / Assistant でコンテンツ型を分けたメッセージ
//! - [`tool`] … ツール使用要求（[`ToolCall`]）と実行結果（[`ToolResult`]）
//! - [`history`] … 不変に扱える会話履歴（[`ConversationHistory`]）
//! - [`turn`] … ツール実行ループの状態遷移（[`Turn`] / [`StopReason`]）
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
pub mod tool;
pub mod turn;

pub use history::ConversationHistory;
pub use message::{AssistantContent, AssistantMessage, Message, UserContent, UserMessage};
pub use tool::{ToolCall, ToolOutcome, ToolResult, ToolUseId};
pub use turn::{StopReason, Turn, TurnError};
