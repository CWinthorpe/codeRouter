use axum::response::sse::{Event, Sse};
use bytes::Bytes;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::task::{Context, Poll};

// ── Anthropic request types ──

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnthropicMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessagesRequest {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub messages: Vec<AnthropicMessage>,
    pub max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

// ── Anthropic response types ──

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnthropicContentBlock {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub content_type: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AnthropicUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MessagesResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub response_type: String,
    pub role: String,
    pub content: Vec<AnthropicContentBlock>,
    pub model: String,
    #[serde(rename = "stop_reason")]
    pub stop_reason: Option<String>,
    #[serde(rename = "stop_sequence")]
    pub stop_sequence: Option<String>,
    pub usage: AnthropicUsage,
}

// ── OpenAI response types ──

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIChoice {
    pub index: u32,
    pub message: OpenAIMessage,
    #[serde(rename = "finish_reason")]
    pub finish_reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
    pub usage: OpenAIUsage,
}

// ── OpenAI streaming chunk types ──

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OpenAIChunkChoice {
    pub index: u32,
    pub delta: OpenAIChunkDelta,
    #[serde(rename = "finish_reason")]
    pub finish_reason: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatCompletionChunk {
    pub id: String,
    pub object: String,
    pub model: String,
    pub choices: Vec<OpenAIChunkChoice>,
}

// ── Anthropic streaming event types ──

#[derive(Deserialize, Debug)]
pub struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub delta: Option<serde_json::Value>,
    #[serde(default)]
    pub message: Option<serde_json::Value>,
    #[serde(default)]
    pub content_block: Option<serde_json::Value>,
    #[serde(default)]
    pub index: Option<u32>,
}

// ── Translation functions ──

pub fn openai_to_anthropic(
    openai_body: &serde_json::Value,
    upstream_model: &str,
) -> MessagesRequest {
    let messages = openai_body
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut system_parts: Vec<String> = Vec::new();
    let mut anthropic_messages: Vec<AnthropicMessage> = Vec::new();

    for msg in &messages {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let content = msg
            .get("content")
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                _ => serde_json::to_string(v).unwrap_or_default(),
            })
            .unwrap_or_default();

        match role {
            "system" => system_parts.push(content),
            "user" => anthropic_messages.push(AnthropicMessage {
                role: "user".to_string(),
                content,
            }),
            "assistant" => anthropic_messages.push(AnthropicMessage {
                role: "assistant".to_string(),
                content,
            }),
            _ => {}
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    let max_tokens = openai_body
        .get("max_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(4096);

    let temperature = openai_body.get("temperature").and_then(|v| v.as_f64());
    let top_p = openai_body.get("top_p").and_then(|v| v.as_f64());
    let stream = openai_body.get("stream").and_then(|v| v.as_bool());

    MessagesRequest {
        model: upstream_model.to_string(),
        system,
        messages: anthropic_messages,
        max_tokens,
        temperature,
        top_p,
        stream,
    }
}

pub fn anthropic_to_openai(
    anthropic_resp: &MessagesResponse,
    group_alias: &str,
) -> ChatCompletionResponse {
    let text = anthropic_resp
        .content
        .first()
        .and_then(|block| block.text.as_deref())
        .unwrap_or("")
        .to_string();

    let finish_reason = match anthropic_resp.stop_reason.as_deref() {
        Some("end_turn") => "stop",
        Some("max_tokens") => "length",
        Some("stop_sequence") => "stop",
        Some("tool_use") => "tool_calls",
        _ => "stop",
    };

    ChatCompletionResponse {
        id: format!("chatcmpl-{}", uuid_short()),
        object: "chat.completion".to_string(),
        created: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        model: group_alias.to_string(),
        choices: vec![OpenAIChoice {
            index: 0,
            message: OpenAIMessage {
                role: "assistant".to_string(),
                content: text,
            },
            finish_reason: finish_reason.to_string(),
        }],
        usage: OpenAIUsage {
            prompt_tokens: anthropic_resp.usage.input_tokens,
            completion_tokens: anthropic_resp.usage.output_tokens,
            total_tokens: anthropic_resp.usage.input_tokens + anthropic_resp.usage.output_tokens,
        },
    }
}

// ── Streaming SSE translator ──

pub struct AnthropicToOpenAIStream<S> {
    inner: S,
    group_alias: String,
    chat_id: String,
    buffer: Vec<u8>,
    state: StreamState,
}

enum StreamState {
    Waiting,
    Done,
}

impl<S> AnthropicToOpenAIStream<S>
where
    S: Stream<Item = Result<bytes::Bytes, std::io::Error>>,
{
    pub fn new(inner: S, group_alias: String) -> Self {
        Self {
            inner,
            group_alias,
            chat_id: format!("chatcmpl-{}", uuid_short()),
            buffer: Vec::new(),
            state: StreamState::Waiting,
        }
    }

    fn translate_event(&self, data: &str) -> Option<String> {
        let event: AnthropicStreamEvent = match serde_json::from_str(data) {
            Ok(e) => e,
            Err(_) => return None,
        };

        match event.event_type.as_str() {
            "message_start" => {
                let chunk = ChatCompletionChunk {
                    id: self.chat_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    model: self.group_alias.clone(),
                    choices: vec![OpenAIChunkChoice {
                        index: 0,
                        delta: OpenAIChunkDelta {
                            role: Some("assistant".to_string()),
                            content: Some("".to_string()),
                        },
                        finish_reason: None,
                    }],
                };
                Some(format!("data: {}\n\n", serde_json::to_string(&chunk).ok()?))
            }
            "content_block_delta" => {
                let text = event
                    .delta
                    .as_ref()
                    .and_then(|d| d.get("text"))
                    .and_then(|v| v.as_str())?;

                if text.is_empty() {
                    return None;
                }

                let chunk = ChatCompletionChunk {
                    id: self.chat_id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    model: self.group_alias.clone(),
                    choices: vec![OpenAIChunkChoice {
                        index: 0,
                        delta: OpenAIChunkDelta {
                            role: None,
                            content: Some(text.to_string()),
                        },
                        finish_reason: None,
                    }],
                };
                Some(format!("data: {}\n\n", serde_json::to_string(&chunk).ok()?))
            }
            "message_delta" => {
                let stop_reason = event
                    .delta
                    .as_ref()
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|v| v.as_str());

                let finish_reason = match stop_reason {
                    Some("end_turn") => Some("stop".to_string()),
                    Some("max_tokens") => Some("length".to_string()),
                    Some("stop_sequence") => Some("stop".to_string()),
                    _ => None,
                };

                if let Some(reason) = finish_reason {
                    let chunk = ChatCompletionChunk {
                        id: self.chat_id.clone(),
                        object: "chat.completion.chunk".to_string(),
                        model: self.group_alias.clone(),
                        choices: vec![OpenAIChunkChoice {
                            index: 0,
                            delta: OpenAIChunkDelta {
                                role: None,
                                content: None,
                            },
                            finish_reason: Some(reason),
                        }],
                    };
                    Some(format!("data: {}\n\n", serde_json::to_string(&chunk).ok()?))
                } else {
                    None
                }
            }
            "message_stop" => Some("data: [DONE]\n\n".to_string()),
            _ => None,
        }
    }
}

impl<S> Stream for AnthropicToOpenAIStream<S>
where
    S: Stream<Item = Result<Bytes, std::io::Error>> + Unpin,
{
    type Item = Result<Event, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if matches!(self.state, StreamState::Done) {
            return Poll::Ready(None);
        }

        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    self.buffer.extend_from_slice(&chunk);

                    if let Some(newline_pos) = self.buffer.iter().position(|&b| b == b'\n') {
                        let line_bytes = self.buffer[..newline_pos].to_vec();
                        self.buffer.drain(..=newline_pos);

                        let line = String::from_utf8_lossy(&line_bytes);
                        let line = line.trim();

                        if line.starts_with("data: ") {
                            let data = &line[6..];
                            if !data.is_empty() {
                                if let Some(output) = self.translate_event(data) {
                                    return Poll::Ready(Some(Ok(Event::default().data(&output))));
                                }
                            }
                        } else if line.starts_with("data:") {
                            let data = &line[5..];
                            if !data.is_empty() {
                                if let Some(output) = self.translate_event(data) {
                                    return Poll::Ready(Some(Ok(Event::default().data(&output))));
                                }
                            }
                        }
                    }
                }
                Poll::Ready(Some(Err(e))) => return Poll::Ready(Some(Err(e))),
                Poll::Ready(None) => {
                    if !self.buffer.is_empty() {
                        let remaining = String::from_utf8_lossy(&self.buffer);
                        let remaining = remaining.trim();
                        if remaining.starts_with("data: ") {
                            let data = &remaining[6..];
                            if !data.is_empty() {
                                if let Some(output) = self.translate_event(data) {
                                    self.buffer.clear();
                                    return Poll::Ready(Some(Ok(Event::default().data(&output))));
                                }
                            }
                        } else if remaining.starts_with("data:") {
                            let data = &remaining[5..];
                            if !data.is_empty() {
                                if let Some(output) = self.translate_event(data) {
                                    self.buffer.clear();
                                    return Poll::Ready(Some(Ok(Event::default().data(&output))));
                                }
                            }
                        }
                        self.buffer.clear();
                    }
                    self.state = StreamState::Done;
                    return Poll::Ready(None);
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

pub fn translate_anthropic_stream<S>(
    stream: S,
    group_alias: String,
) -> Sse<AnthropicToOpenAIStream<S>>
where
    S: Stream<Item = Result<bytes::Bytes, std::io::Error>> + Send + Unpin + 'static,
{
    let translated = AnthropicToOpenAIStream::new(stream, group_alias);
    Sse::new(translated)
}

// ── HTTP headers helper ──

pub fn anthropic_headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("x-api-key".to_string(), api_key.to_string()),
        ("anthropic-version".to_string(), "2024-06-01".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
    ]
}

// ── UUID helper ──

fn uuid_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ts = duration.as_millis();
    let rand = fastrand::u64(..);
    format!("{:x}{:x}", ts, rand & 0xFFFFFFFF)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_to_anthropic_basic() {
        let openai_req = serde_json::json!({
            "model": "test-model",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ],
            "max_tokens": 100,
            "temperature": 0.7,
            "stream": true
        });

        let result = openai_to_anthropic(&openai_req, "claude-sonnet-4-20250514");

        assert_eq!(result.model, "claude-sonnet-4-20250514");
        assert_eq!(result.system, Some("You are helpful.".to_string()));
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].role, "user");
        assert_eq!(result.messages[0].content, "Hello");
        assert_eq!(result.max_tokens, 100);
        assert_eq!(result.temperature, Some(0.7));
        assert_eq!(result.stream, Some(true));
    }

    #[test]
    fn test_openai_to_anthropic_multiple_system_messages() {
        let openai_req = serde_json::json!({
            "messages": [
                {"role": "system", "content": "Part 1"},
                {"role": "system", "content": "Part 2"},
                {"role": "user", "content": "Hi"},
                {"role": "assistant", "content": "Hey"},
                {"role": "user", "content": "Bye"}
            ]
        });

        let result = openai_to_anthropic(&openai_req, "claude-model");

        assert_eq!(result.system, Some("Part 1\n\nPart 2".to_string()));
        assert_eq!(result.messages.len(), 3);
        assert_eq!(result.messages[0].role, "user");
        assert_eq!(result.messages[1].role, "assistant");
        assert_eq!(result.messages[2].role, "user");
    }

    #[test]
    fn test_openai_to_anthropic_default_max_tokens() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}]
        });

        let result = openai_to_anthropic(&openai_req, "model");

        assert_eq!(result.max_tokens, 4096);
    }

    #[test]
    fn test_openai_to_anthropic_drops_unsupported_fields() {
        let openai_req = serde_json::json!({
            "messages": [{"role": "user", "content": "test"}],
            "n": 2,
            "logprobs": true,
            "presence_penalty": 0.5
        });

        let result = openai_to_anthropic(&openai_req, "model");

        assert!(result.temperature.is_none());
        assert!(result.top_p.is_none());
    }

    #[test]
    fn test_anthropic_to_openai_non_streaming() {
        let anthropic_resp = MessagesResponse {
            id: "msg_abc123".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![AnthropicContentBlock {
                text: Some("Hello, world!".to_string()),
                content_type: "text".to_string(),
            }],
            model: "claude-sonnet-4-20250514".to_string(),
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
            },
        };

        let result = anthropic_to_openai(&anthropic_resp, "my-group");

        assert_eq!(result.object, "chat.completion");
        assert_eq!(result.model, "my-group");
        assert_eq!(result.choices.len(), 1);
        assert_eq!(result.choices[0].message.role, "assistant");
        assert_eq!(result.choices[0].message.content, "Hello, world!");
        assert_eq!(result.choices[0].finish_reason, "stop");
        assert_eq!(result.usage.prompt_tokens, 10);
        assert_eq!(result.usage.completion_tokens, 5);
        assert_eq!(result.usage.total_tokens, 15);
    }

    #[test]
    fn test_anthropic_to_openai_max_tokens_stop() {
        let anthropic_resp = MessagesResponse {
            id: "msg_xyz".to_string(),
            response_type: "message".to_string(),
            role: "assistant".to_string(),
            content: vec![AnthropicContentBlock {
                text: Some("truncated".to_string()),
                content_type: "text".to_string(),
            }],
            model: "claude".to_string(),
            stop_reason: Some("max_tokens".to_string()),
            stop_sequence: None,
            usage: AnthropicUsage {
                input_tokens: 100,
                output_tokens: 4096,
            },
        };

        let result = anthropic_to_openai(&anthropic_resp, "group");

        assert_eq!(result.choices[0].finish_reason, "length");
    }

    #[test]
    fn test_anthropic_headers() {
        let headers = anthropic_headers("sk-ant-test-key");

        assert_eq!(headers.len(), 3);
        assert_eq!(
            headers[0],
            ("x-api-key".to_string(), "sk-ant-test-key".to_string())
        );
        assert_eq!(
            headers[1],
            ("anthropic-version".to_string(), "2024-06-01".to_string())
        );
        assert_eq!(
            headers[2],
            ("content-type".to_string(), "application/json".to_string())
        );
    }

    #[test]
    fn test_stream_translation_message_start() {
        let mut streamer =
            AnthropicToOpenAIStream::new(futures::stream::empty(), "test-group".to_string());
        streamer.chat_id = "chatcmpl-test".to_string();

        let data = r#"{"type":"message_start","message":{"id":"msg_123"}}"#;
        let result = streamer.translate_event(data).unwrap();

        assert!(result.contains("chatcmpl-test"));
        assert!(result.contains("chat.completion.chunk"));
        assert!(result.contains("test-group"));
        assert!(result.contains("\"role\":\"assistant\""));
    }

    #[test]
    fn test_stream_translation_content_delta() {
        let mut streamer =
            AnthropicToOpenAIStream::new(futures::stream::empty(), "test-group".to_string());
        streamer.chat_id = "chatcmpl-test".to_string();

        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let result = streamer.translate_event(data).unwrap();

        assert!(result.contains("\"content\":\"Hello\""));
        assert!(result.contains("chatcmpl-test"));
    }

    #[test]
    fn test_stream_translation_message_delta_stop() {
        let mut streamer =
            AnthropicToOpenAIStream::new(futures::stream::empty(), "test-group".to_string());
        streamer.chat_id = "chatcmpl-test".to_string();

        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":10}}"#;
        let result = streamer.translate_event(data).unwrap();

        assert!(result.contains("\"finish_reason\":\"stop\""));
    }

    #[test]
    fn test_stream_translation_message_stop() {
        let streamer =
            AnthropicToOpenAIStream::new(futures::stream::empty(), "test-group".to_string());

        let data = r#"{"type":"message_stop"}"#;
        let result = streamer.translate_event(data).unwrap();

        assert!(result.contains("[DONE]"));
    }

    #[test]
    fn test_stream_translation_unknown_event_skipped() {
        let streamer =
            AnthropicToOpenAIStream::new(futures::stream::empty(), "test-group".to_string());

        let data = r#"{"type":"ping"}"#;
        let result = streamer.translate_event(data);

        assert!(result.is_none());
    }
}
