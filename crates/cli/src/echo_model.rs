use agent_core::{
    message::{AssistantMessage, Message, UserContent},
    ports::{LanguageModel, ModelError, ModelResponse},
    turn::StopReason,
    ConversationHistory,
};
use async_trait::async_trait;

/// ユーザーの最新テキストをオウム返しするデモ用モデル。
///
/// 常に `StopReason::EndTurn` を返すため、ツール実行ループには入らない。
/// REPL の動作確認に使う。
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

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        message::{AssistantContent, UserMessage},
        tool::{ToolResult, ToolUseId},
    };
    use serde_json::json;

    #[tokio::test]
    async fn echoes_single_user_message() {
        let history = ConversationHistory::new().with(UserMessage::text("hello"));
        let response = EchoLanguageModel.complete(&history).await.unwrap();
        assert_eq!(response.message, AssistantMessage::text("Echo: hello"));
    }

    #[tokio::test]
    async fn echoes_latest_user_text_across_multiple_turns() {
        let history = ConversationHistory::new()
            .with(UserMessage::text("first"))
            .with(AssistantMessage::text("Echo: first"))
            .with(UserMessage::text("second"));
        let response = EchoLanguageModel.complete(&history).await.unwrap();
        assert_eq!(response.message, AssistantMessage::text("Echo: second"));
    }

    #[tokio::test]
    async fn falls_back_on_empty_history() {
        let history = ConversationHistory::new();
        let response = EchoLanguageModel.complete(&history).await.unwrap();
        assert_eq!(response.message, AssistantMessage::text("Echo: ..."));
    }

    #[tokio::test]
    async fn skips_tool_result_only_messages() {
        let tool_result_msg =
            UserMessage::tool_results(vec![ToolResult::success(ToolUseId::new("t1"), json!(1))]);
        let history = ConversationHistory::new()
            .with(UserMessage::text("original"))
            .with(AssistantMessage::text("Echo: original"))
            .with(tool_result_msg);
        let response = EchoLanguageModel.complete(&history).await.unwrap();
        assert_eq!(response.message, AssistantMessage::text("Echo: original"));
    }

    #[tokio::test]
    async fn returns_end_turn_stop_reason() {
        let history = ConversationHistory::new().with(UserMessage::text("hi"));
        let response = EchoLanguageModel.complete(&history).await.unwrap();
        if let StopReason::EndTurn(msg) = &response.stop_reason {
            assert_eq!(
                msg.content,
                vec![AssistantContent::Text("Echo: hi".to_string())]
            );
        } else {
            panic!("expected EndTurn");
        }
    }
}
