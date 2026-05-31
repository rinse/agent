//! 会話履歴。エージェントの「作業記憶」であり、各ターンの蓄積がモデルの文脈になる。
//!
//! 設計方針に従い、履歴は**不変に扱う**ことを基本とする。メッセージの追加は
//! 「新しい履歴を返す」関数 [`ConversationHistory::with`] として表現でき、
//! ループ駆動部を「状態 → 新しい状態」の純粋な変換として書ける。
//! 利便性のため、その場で変更する [`ConversationHistory::push`] も用意する。

use serde::{Deserialize, Serialize};

use crate::message::Message;

/// 時系列に並んだメッセージ列。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ConversationHistory {
    messages: Vec<Message>,
}

impl ConversationHistory {
    /// 空の履歴を作る。
    pub fn new() -> Self {
        Self::default()
    }

    /// 既存のメッセージ列から履歴を組み立てる。
    pub fn from_messages(messages: impl IntoIterator<Item = Message>) -> Self {
        Self {
            messages: messages.into_iter().collect(),
        }
    }

    /// メッセージを 1 件追加した**新しい履歴**を返す（関数型スタイル／元の履歴は不変）。
    ///
    /// 履歴を「状態 → 新しい状態」として組み立てたい場面（ターンの構築・テスト・
    /// 分岐させて複数案を保持したいとき）で使う。一方、ループ内で同じ履歴へ繰り返し
    /// 追記する経路では、毎回の move を避けられる [`ConversationHistory::push`] が向く。
    /// 二系統あるのは用途の違いによるもので、得られる結果は等価。
    #[must_use]
    pub fn with(mut self, message: impl Into<Message>) -> Self {
        self.messages.push(message.into());
        self
    }

    /// メッセージをその場で追加する（命令型スタイル）。
    ///
    /// `impl Into<Message>` を取るので、[`UserMessage`] / [`AssistantMessage`] を
    /// そのまま渡せる（`From` 実装により [`Message`] へ変換される）。ロール別の
    /// 薄いラッパは利用箇所が出てから必要に応じて足す方針とし、ここでは増やさない。
    pub fn push(&mut self, message: impl Into<Message>) {
        self.messages.push(message.into());
    }

    /// メッセージ列を借用で取り出す。
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// メッセージを古い順に走査する。
    pub fn iter(&self) -> std::slice::Iter<'_, Message> {
        self.messages.iter()
    }

    /// 直近のメッセージ。空なら `None`。
    pub fn last(&self) -> Option<&Message> {
        self.messages.last()
    }

    /// メッセージ件数。
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// 履歴が空かどうか。
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

impl<'a> IntoIterator for &'a ConversationHistory {
    type Item = &'a Message;
    type IntoIter = std::slice::Iter<'a, Message>;

    fn into_iter(self) -> Self::IntoIter {
        self.messages.iter()
    }
}

impl FromIterator<Message> for ConversationHistory {
    fn from_iter<T: IntoIterator<Item = Message>>(iter: T) -> Self {
        Self::from_messages(iter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{AssistantMessage, UserMessage};

    #[test]
    fn new_history_is_empty() {
        let history = ConversationHistory::new();
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.last(), None);
    }

    #[test]
    fn with_returns_new_history_and_leaves_original_untouched() {
        let original = ConversationHistory::new();
        let next = original.clone().with(UserMessage::text("hi"));

        // 元の履歴は不変。
        assert!(original.is_empty());
        // 新しい履歴に 1 件だけ追加されている。
        assert_eq!(next.len(), 1);
        assert_eq!(next.last(), Some(&Message::User(UserMessage::text("hi"))));
    }

    #[test]
    fn with_can_be_chained_to_build_a_turn() {
        let history = ConversationHistory::new()
            .with(UserMessage::text("question"))
            .with(AssistantMessage::text("answer"));

        assert_eq!(history.len(), 2);
        assert!(matches!(history.messages()[0], Message::User(_)));
        assert!(matches!(history.messages()[1], Message::Assistant(_)));
    }

    #[test]
    fn push_mutates_in_place() {
        let mut history = ConversationHistory::new();
        // `push` は `impl Into<Message>` を取るので両ロールをそのまま渡せる。
        history.push(UserMessage::text("hi"));
        history.push(AssistantMessage::text("hello"));
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn iterates_in_insertion_order() {
        let history = ConversationHistory::new()
            .with(UserMessage::text("a"))
            .with(AssistantMessage::text("b"));

        let roles: Vec<&str> = history
            .iter()
            .map(|m| match m {
                Message::User(_) => "user",
                Message::Assistant(_) => "assistant",
            })
            .collect();
        assert_eq!(roles, vec!["user", "assistant"]);
    }

    #[test]
    fn collects_from_message_iterator() {
        let messages = vec![
            Message::from(UserMessage::text("a")),
            Message::from(AssistantMessage::text("b")),
        ];
        let history: ConversationHistory = messages.clone().into_iter().collect();
        assert_eq!(history.messages(), messages.as_slice());
    }

    #[test]
    fn history_roundtrips_through_serde() {
        let history = ConversationHistory::new()
            .with(UserMessage::text("hi"))
            .with(AssistantMessage::text("hello"));
        let encoded = serde_json::to_string(&history).unwrap();
        let decoded: ConversationHistory = serde_json::from_str(&encoded).unwrap();
        assert_eq!(history, decoded);
    }
}
