//! OpenAI 互換 API を通じて LLM を呼び出すアダプタ。
//!
//! LM Studio などの OpenAI 互換サーバと通信する。
//! デフォルトのベース URL は `http://localhost:1234/v1`（LM Studio の既定値）。

use agent_core::{
    message::{AssistantContent, AssistantMessage, Message, UserContent},
    ports::{LanguageModel, ModelError, ModelResponse},
    tool::{ToolCall, ToolOutcome, ToolUseId},
    turn::StopReason,
    ConversationHistory,
};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// モデルに送信するツール定義。
///
/// OpenAI 互換 API の `tools` パラメータに対応する。
/// `parameters` は JSON Schema 形式で指定する。
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

pub struct OpenAiModel {
    client: Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
    system_prompt: Option<String>,
    tools: Vec<ToolDefinition>,
}

impl OpenAiModel {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            system_prompt: None,
            tools: Vec::new(),
        }
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    fn build_messages(&self, history: &ConversationHistory) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        if let Some(system) = &self.system_prompt {
            messages.push(ChatMessage {
                role: "system".into(),
                content: Some(system.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        for msg in history.messages() {
            match msg {
                Message::User(user_msg) => {
                    let mut texts = Vec::new();
                    let mut tool_results = Vec::new();

                    for content in &user_msg.content {
                        match content {
                            UserContent::Text(t) => texts.push(t.as_str()),
                            UserContent::ToolResult(r) => tool_results.push(r),
                        }
                    }

                    if !texts.is_empty() {
                        messages.push(ChatMessage {
                            role: "user".into(),
                            content: Some(texts.join("\n")),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }

                    for result in tool_results {
                        let content = match &result.outcome {
                            ToolOutcome::Success(v) => {
                                serde_json::to_string(v)
                                    .expect("Value serialization is infallible")
                            }
                            ToolOutcome::Error(e) => format!("Error: {e}"),
                        };
                        messages.push(ChatMessage {
                            role: "tool".into(),
                            content: Some(content),
                            tool_calls: None,
                            tool_call_id: Some(result.tool_use_id.as_str().to_string()),
                        });
                    }
                }
                Message::Assistant(asst_msg) => {
                    let mut texts = Vec::new();
                    let mut calls = Vec::new();

                    for content in &asst_msg.content {
                        match content {
                            AssistantContent::Text(t) => texts.push(t.as_str()),
                            AssistantContent::ToolUse(call) => {
                                calls.push(OaiToolCall {
                                    id: call.id.as_str().to_string(),
                                    call_type: "function".into(),
                                    function: OaiFunction {
                                        name: call.name.clone(),
                                        arguments: serde_json::to_string(&call.input)
                                            .expect("Value serialization is infallible"),
                                    },
                                });
                            }
                        }
                    }

                    messages.push(ChatMessage {
                        role: "assistant".into(),
                        content: if texts.is_empty() {
                            None
                        } else {
                            Some(texts.join("\n"))
                        },
                        tool_calls: if calls.is_empty() { None } else { Some(calls) },
                        tool_call_id: None,
                    });
                }
            }
        }

        messages
    }
}

#[async_trait]
impl LanguageModel for OpenAiModel {
    async fn complete(&self, history: &ConversationHistory) -> Result<ModelResponse, ModelError> {
        let messages = self.build_messages(history);

        let tools = if self.tools.is_empty() {
            None
        } else {
            Some(
                self.tools
                    .iter()
                    .map(|t| OaiToolDefinition {
                        tool_type: "function".into(),
                        function: OaiToolFunction {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            parameters: t.parameters.clone(),
                        },
                    })
                    .collect(),
            )
        };

        let request = ChatCompletionRequest {
            model: self.model.clone(),
            messages,
            tools,
        };

        let url = format!(
            "{}/chat/completions",
            self.base_url.trim_end_matches('/')
        );

        let mut req = self.client.post(&url).json(&request);

        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let response = req
            .send()
            .await
            .map_err(|e| ModelError::new(format!("HTTP request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ModelError::new(format!("API error {status}: {body}")));
        }

        let completion: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| ModelError::new(format!("failed to parse response: {e}")))?;

        parse_response(completion)
    }
}

fn parse_response(response: ChatCompletionResponse) -> Result<ModelResponse, ModelError> {
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| ModelError::new("empty choices in response"))?;

    let mut content = Vec::new();

    if let Some(text) = &choice.message.content {
        if !text.is_empty() {
            content.push(AssistantContent::Text(text.clone()));
        }
    }

    if let Some(calls) = choice.message.tool_calls {
        for call in calls {
            let input: serde_json::Value =
                serde_json::from_str(&call.function.arguments).map_err(|e| {
                    ModelError::new(format!(
                        "failed to parse tool arguments for '{}': {e}",
                        call.function.name
                    ))
                })?;
            content.push(AssistantContent::ToolUse(ToolCall::new(
                ToolUseId::new(call.id),
                call.function.name,
                input,
            )));
        }
    }

    let message = AssistantMessage { content };
    let tool_calls: Vec<ToolCall> = message.tool_calls().cloned().collect();

    let finish_reason = choice.finish_reason.unwrap_or_default();
    let stop_reason = match finish_reason.as_str() {
        "tool_calls" if !tool_calls.is_empty() => StopReason::ToolUse(tool_calls),
        "length" => StopReason::MaxTokens,
        _ => {
            if !tool_calls.is_empty() {
                StopReason::ToolUse(tool_calls)
            } else {
                StopReason::EndTurn(message.clone())
            }
        }
    };

    Ok(ModelResponse {
        message,
        stop_reason,
    })
}

// ─────────────────────────────────────────────
// OpenAI API wire types
// ─────────────────────────────────────────────

#[derive(Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OaiToolDefinition>>,
}

#[derive(Serialize)]
struct OaiToolDefinition {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiToolFunction,
}

#[derive(Serialize)]
struct OaiToolFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunction,
}

#[derive(Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        message::{AssistantMessage, UserMessage},
        tool::{ToolResult, ToolUseId},
    };
    use serde_json::json;

    #[test]
    fn builds_user_text_message() {
        let model = OpenAiModel::new("http://localhost:1234/v1", "test");
        let history = ConversationHistory::new().with(UserMessage::text("hello"));
        let messages = model.build_messages(&history);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content.as_deref(), Some("hello"));
    }

    #[test]
    fn builds_system_prompt_first() {
        let model =
            OpenAiModel::new("http://localhost:1234/v1", "test").with_system_prompt("Be helpful.");
        let history = ConversationHistory::new().with(UserMessage::text("hi"));
        let messages = model.build_messages(&history);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "system");
        assert_eq!(messages[0].content.as_deref(), Some("Be helpful."));
        assert_eq!(messages[1].role, "user");
    }

    #[test]
    fn builds_assistant_text_message() {
        let model = OpenAiModel::new("http://localhost:1234/v1", "test");
        let history = ConversationHistory::new()
            .with(UserMessage::text("hi"))
            .with(AssistantMessage::text("hello"));
        let messages = model.build_messages(&history);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content.as_deref(), Some("hello"));
        assert!(messages[1].tool_calls.is_none());
    }

    #[test]
    fn builds_assistant_tool_use_message() {
        let model = OpenAiModel::new("http://localhost:1234/v1", "test");
        let call = ToolCall::new(ToolUseId::new("c1"), "search", json!({"q": "rust"}));
        let asst = AssistantMessage {
            content: vec![
                AssistantContent::Text("let me search".into()),
                AssistantContent::ToolUse(call),
            ],
        };
        let history = ConversationHistory::new()
            .with(UserMessage::text("search rust"))
            .with(asst);
        let messages = model.build_messages(&history);

        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content.as_deref(), Some("let me search"));
        let tool_calls = messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].id, "c1");
        assert_eq!(tool_calls[0].function.name, "search");
    }

    #[test]
    fn builds_tool_result_messages() {
        let model = OpenAiModel::new("http://localhost:1234/v1", "test");
        let results = vec![
            ToolResult::success(ToolUseId::new("c1"), json!({"found": true})),
            ToolResult::error(ToolUseId::new("c2"), "not found"),
        ];
        let history = ConversationHistory::new().with(UserMessage::tool_results(results));
        let messages = model.build_messages(&history);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "tool");
        assert_eq!(messages[0].tool_call_id.as_deref(), Some("c1"));
        assert_eq!(
            messages[0].content.as_deref(),
            Some("{\"found\":true}")
        );
        assert_eq!(messages[1].role, "tool");
        assert_eq!(messages[1].tool_call_id.as_deref(), Some("c2"));
        assert_eq!(messages[1].content.as_deref(), Some("Error: not found"));
    }

    #[test]
    fn parses_end_turn_response() {
        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: Some("Hello!".into()),
                    tool_calls: None,
                },
                finish_reason: Some("stop".into()),
            }],
        };
        let result = parse_response(response).unwrap();
        assert_eq!(result.message, AssistantMessage::text("Hello!"));
        assert!(matches!(result.stop_reason, StopReason::EndTurn(_)));
    }

    #[test]
    fn parses_tool_calls_response() {
        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_1".into(),
                        call_type: "function".into(),
                        function: OaiFunction {
                            name: "search".into(),
                            arguments: r#"{"q":"rust"}"#.into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        };
        let result = parse_response(response).unwrap();
        assert!(result.message.requests_tools());
        if let StopReason::ToolUse(calls) = &result.stop_reason {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].name, "search");
            assert_eq!(calls[0].input, json!({"q": "rust"}));
        } else {
            panic!("expected ToolUse stop reason");
        }
    }

    #[test]
    fn parses_max_tokens_response() {
        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: Some("partial...".into()),
                    tool_calls: None,
                },
                finish_reason: Some("length".into()),
            }],
        };
        let result = parse_response(response).unwrap();
        assert!(matches!(result.stop_reason, StopReason::MaxTokens));
    }

    #[test]
    fn empty_choices_is_error() {
        let response = ChatCompletionResponse { choices: vec![] };
        assert!(parse_response(response).is_err());
    }

    #[test]
    fn invalid_tool_arguments_returns_error() {
        let response = ChatCompletionResponse {
            choices: vec![Choice {
                message: ResponseMessage {
                    content: None,
                    tool_calls: Some(vec![OaiToolCall {
                        id: "c1".into(),
                        call_type: "function".into(),
                        function: OaiFunction {
                            name: "search".into(),
                            arguments: "not valid json".into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }],
        };
        let result = parse_response(response);
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("search"));
    }

    #[test]
    fn request_includes_tools_when_defined() {
        let tools = vec![ToolDefinition {
            name: "search".into(),
            description: "Search the web".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                },
                "required": ["query"]
            }),
        }];
        let model = OpenAiModel::new("http://localhost:1234/v1", "test").with_tools(tools);

        // tools フィールドを含むリクエストが構築されることをシリアライズで検証
        let history = ConversationHistory::new().with(UserMessage::text("hi"));
        let messages = model.build_messages(&history);
        let oai_tools: Vec<OaiToolDefinition> = model
            .tools
            .iter()
            .map(|t| OaiToolDefinition {
                tool_type: "function".into(),
                function: OaiToolFunction {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                },
            })
            .collect();
        let request = ChatCompletionRequest {
            model: "test".into(),
            messages,
            tools: Some(oai_tools),
        };
        let json = serde_json::to_value(&request).unwrap();
        let tools_arr = json["tools"].as_array().unwrap();
        assert_eq!(tools_arr.len(), 1);
        assert_eq!(tools_arr[0]["type"], "function");
        assert_eq!(tools_arr[0]["function"]["name"], "search");
    }

    #[test]
    fn request_omits_tools_when_empty() {
        let model = OpenAiModel::new("http://localhost:1234/v1", "test");
        let history = ConversationHistory::new().with(UserMessage::text("hi"));
        let messages = model.build_messages(&history);
        let request = ChatCompletionRequest {
            model: "test".into(),
            messages,
            tools: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("tools").is_none());
    }
}
