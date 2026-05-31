//! ツール呼び出しに関する基本型。
//!
//! エージェントループでは、アシスタントが「ツールを使う」と判断すると [`ToolCall`] を発行し、
//! システムがそれを実行した結果を [`ToolResult`] として会話履歴へ戻す（フィードバックループ）。
//! ここで定義するのは**履歴に載るデータ**だけで、実行そのもの（`Tool` トレイト等）は別レイヤの責務。
//!
//! 重要な設計上の不変条件: **ツールの失敗はループを止めない**。失敗は `Result::Err` として
//! ループ外に投げるのではなく、[`ToolOutcome::Error`] という「正常な結果値」としてモデルへ戻す。
//! これによりモデルが回復を試みられる。

use serde::{Deserialize, Serialize};

/// ツール使用要求と、その結果を対応づけるための識別子。
///
/// アシスタントが発行した [`ToolCall::id`] と、戻ってくる [`ToolResult::tool_use_id`] が
/// 一致することで、どの要求に対する結果かを一意に紐づける。新しい型（newtype）にすることで、
/// ただの `String` と取り違えるミスをコンパイル時に防ぐ。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolUseId(String);

impl ToolUseId {
    /// 任意の文字列から識別子を作る。
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// 内部の文字列表現を借用で取り出す。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ToolUseId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for ToolUseId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl std::fmt::Display for ToolUseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// アシスタントが発行する、1 件のツール実行要求。
///
/// 入力は LLM / MCP のいずれでも本質的に JSON なので [`serde_json::Value`] で保持する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// この要求と結果を対応づける識別子。
    pub id: ToolUseId,
    /// 呼び出すツール名。
    pub name: String,
    /// ツールへ渡す入力（任意の JSON 値）。
    ///
    /// コア型をあえて構造化せず [`serde_json::Value`] に留めるのは、LLM / MCP の
    /// いずれもツール入力が本質的に JSON だから。入力をツールごとのスキーマに照らして
    /// **検証する**のは、ツールを解決・実行するレイヤ（後続タスク）の責務とし、
    /// ここでは構造化された値の運搬だけを担う。こうしておけば、後でスキーマ検証を
    /// 足してもコア型は変えずに済む。
    pub input: serde_json::Value,
}

impl ToolCall {
    /// ツール呼び出しを組み立てる。
    pub fn new(id: ToolUseId, name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            id,
            name: name.into(),
            input,
        }
    }
}

/// ツール実行の結末。成功も失敗も**値として**表現する。
///
/// 失敗を `Err` でループ外へ投げず [`ToolOutcome::Error`] に閉じ込めることで、
/// 「ツールが失敗してもループは継続しモデルへ結果を返す」という不変条件を型に刻む。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolOutcome {
    /// 正常終了。出力は任意の JSON 値。
    Success(serde_json::Value),
    /// 失敗。モデルが回復を試みられるよう、人間可読なメッセージを添える。
    Error(String),
}

impl ToolOutcome {
    /// 失敗（[`ToolOutcome::Error`]）かどうか。
    pub fn is_error(&self) -> bool {
        matches!(self, ToolOutcome::Error(_))
    }
}

/// ツール実行の結果。会話履歴では User 側のコンテンツとしてモデルへ戻る。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    /// どの [`ToolCall`] に対する結果か。
    pub tool_use_id: ToolUseId,
    /// 実行の結末（成功／失敗）。
    pub outcome: ToolOutcome,
}

impl ToolResult {
    /// 成功結果を作る。
    pub fn success(tool_use_id: ToolUseId, output: serde_json::Value) -> Self {
        Self {
            tool_use_id,
            outcome: ToolOutcome::Success(output),
        }
    }

    /// 失敗結果を作る。
    pub fn error(tool_use_id: ToolUseId, message: impl Into<String>) -> Self {
        Self {
            tool_use_id,
            outcome: ToolOutcome::Error(message.into()),
        }
    }

    /// この結果が失敗かどうか。
    pub fn is_error(&self) -> bool {
        self.outcome.is_error()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_use_id_roundtrips_through_str() {
        let id = ToolUseId::new("call_42");
        assert_eq!(id.as_str(), "call_42");
        assert_eq!(id.to_string(), "call_42");
    }

    #[test]
    fn tool_use_id_from_str_and_string_match_new() {
        let expected = ToolUseId::new("c1");
        assert_eq!(ToolUseId::from("c1"), expected);
        assert_eq!(ToolUseId::from("c1".to_string()), expected);
        // `.into()` でも書ける（呼び出し側の取り回し用）。
        let via_into: ToolUseId = "c1".into();
        assert_eq!(via_into, expected);
    }

    #[test]
    fn success_result_is_not_error() {
        let result = ToolResult::success(ToolUseId::new("c1"), json!({"ok": true}));
        assert!(!result.is_error());
        assert_eq!(result.outcome, ToolOutcome::Success(json!({"ok": true})));
    }

    #[test]
    fn error_result_is_error() {
        let result = ToolResult::error(ToolUseId::new("c1"), "boom");
        assert!(result.is_error());
        assert_eq!(result.outcome, ToolOutcome::Error("boom".to_string()));
    }

    #[test]
    fn tool_call_serializes_to_expected_json() {
        let call = ToolCall::new(ToolUseId::new("c1"), "search", json!({"q": "rust"}));
        let value = serde_json::to_value(&call).unwrap();
        assert_eq!(
            value,
            json!({"id": "c1", "name": "search", "input": {"q": "rust"}})
        );
    }

    #[test]
    fn tool_call_roundtrips_through_serde() {
        let call = ToolCall::new(ToolUseId::new("c1"), "search", json!({"q": "rust"}));
        let encoded = serde_json::to_string(&call).unwrap();
        let decoded: ToolCall = serde_json::from_str(&encoded).unwrap();
        assert_eq!(call, decoded);
    }
}
