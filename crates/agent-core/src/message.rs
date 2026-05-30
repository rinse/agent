//! 会話メッセージの基本型。
//!
//! エージェントループでやり取りされるメッセージは、発話者（ロール）によって**載せられる内容が違う**:
//!
//! - **User** … テキスト、そして「ツール実行結果」をモデルへ戻すフィードバック
//! - **Assistant** … テキスト、そして「ツール使用要求」
//!
//! これを 1 つの平坦な `enum` にまとめると「アシスタントがツール結果を持つ」「ユーザーがツール要求を出す」
//! といった**ありえない状態**が表現可能になってしまう。そこで本モジュールでは User / Assistant ごとに
//! コンテンツ型（[`UserContent`] / [`AssistantContent`]）を分け、不正な組み合わせをコンパイル時に排除する。

use serde::{Deserialize, Serialize};

use crate::tool::{ToolCall, ToolResult};

/// User メッセージに載せられるコンテンツ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UserContent {
    /// ユーザー（または上流システム）からのテキスト。
    Text(String),
    /// 直前のアシスタントのツール使用要求に対する実行結果。
    ToolResult(ToolResult),
}

/// Assistant メッセージに載せられるコンテンツ。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssistantContent {
    /// モデルが生成したテキスト応答。
    Text(String),
    /// モデルが要求するツール実行。
    ToolUse(ToolCall),
}

/// ユーザー発のメッセージ。複数コンテンツ（テキスト＋複数のツール結果など）を持てる。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct UserMessage {
    /// 出現順に並んだコンテンツ。
    pub content: Vec<UserContent>,
}

impl UserMessage {
    /// 空のメッセージを作る。
    pub fn new() -> Self {
        Self::default()
    }

    /// 単一テキストのメッセージを作る（最も一般的なユーザー入力）。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![UserContent::Text(text.into())],
        }
    }

    /// 1 件以上のツール結果からメッセージを作る（ツール実行ループのフィードバック）。
    pub fn tool_results(results: impl IntoIterator<Item = ToolResult>) -> Self {
        Self {
            content: results.into_iter().map(UserContent::ToolResult).collect(),
        }
    }
}

/// アシスタント発のメッセージ。テキストとツール使用要求が混在しうる。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AssistantMessage {
    /// 出現順に並んだコンテンツ。
    pub content: Vec<AssistantContent>,
}

impl AssistantMessage {
    /// 空のメッセージを作る。
    pub fn new() -> Self {
        Self::default()
    }

    /// 単一テキストの応答を作る。
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![AssistantContent::Text(text.into())],
        }
    }

    /// このメッセージが要求するツール呼び出しを出現順に列挙する。
    ///
    /// ツール実行ループの駆動部が「次に走らせる呼び出し」を取り出すのに使う。
    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.content.iter().filter_map(|c| match c {
            AssistantContent::ToolUse(call) => Some(call),
            AssistantContent::Text(_) => None,
        })
    }

    /// ツール使用要求を 1 件でも含むか。
    pub fn requests_tools(&self) -> bool {
        self.tool_calls().next().is_some()
    }
}

/// 会話履歴に並ぶ 1 件のメッセージ。ロールによって内容の型が変わる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// ユーザー発のメッセージ。
    User(UserMessage),
    /// アシスタント発のメッセージ。
    Assistant(AssistantMessage),
}

impl From<UserMessage> for Message {
    fn from(message: UserMessage) -> Self {
        Message::User(message)
    }
}

impl From<AssistantMessage> for Message {
    fn from(message: AssistantMessage) -> Self {
        Message::Assistant(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolResult, ToolUseId};
    use serde_json::json;

    #[test]
    fn user_text_holds_single_text_block() {
        let msg = UserMessage::text("hello");
        assert_eq!(msg.content, vec![UserContent::Text("hello".to_string())]);
    }

    #[test]
    fn user_tool_results_maps_each_result() {
        let results = vec![
            ToolResult::success(ToolUseId::new("a"), json!(1)),
            ToolResult::error(ToolUseId::new("b"), "nope"),
        ];
        let msg = UserMessage::tool_results(results.clone());
        let expected: Vec<UserContent> = results.into_iter().map(UserContent::ToolResult).collect();
        assert_eq!(msg.content, expected);
    }

    #[test]
    fn assistant_tool_calls_filters_only_tool_uses() {
        let call = ToolCall::new(ToolUseId::new("c1"), "search", json!({}));
        let msg = AssistantMessage {
            content: vec![
                AssistantContent::Text("let me look".to_string()),
                AssistantContent::ToolUse(call.clone()),
            ],
        };
        assert!(msg.requests_tools());
        let calls: Vec<&ToolCall> = msg.tool_calls().collect();
        assert_eq!(calls, vec![&call]);
    }

    #[test]
    fn assistant_plain_text_requests_no_tools() {
        let msg = AssistantMessage::text("done");
        assert!(!msg.requests_tools());
        assert_eq!(msg.tool_calls().count(), 0);
    }

    #[test]
    fn message_from_conversions() {
        assert!(matches!(
            Message::from(UserMessage::text("hi")),
            Message::User(_)
        ));
        assert!(matches!(
            Message::from(AssistantMessage::text("yo")),
            Message::Assistant(_)
        ));
    }
}
