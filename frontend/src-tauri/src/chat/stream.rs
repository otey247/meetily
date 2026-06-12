use futures_util::StreamExt;
use reqwest::{header, Client};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::summary::llm_client::LLMProvider;

const REQUEST_TIMEOUT_DURATION: Duration = Duration::from_secs(300);
const CLAUDE_MAX_TOKENS: u32 = 2048;

/// One turn of a chat conversation sent to the LLM.
#[derive(Debug, Clone, Serialize)]
pub struct ChatTurn {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct StreamingChatRequest {
    model: String,
    messages: Vec<ChatTurn>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

#[derive(Debug, Serialize)]
struct ClaudeStreamingRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ChatTurn>,
    stream: bool,
}

/// Result of parsing one SSE line.
#[derive(Debug, PartialEq)]
enum SseEvent {
    Delta(String),
    Done,
    Ignore,
    Error(String),
}

/// Accumulates raw network bytes and yields complete lines. SSE `data:` lines
/// can be split across network chunks, so partial lines stay buffered. Splits
/// on b'\n', which is safe even with multi-byte UTF-8 (continuation bytes are
/// always >= 0x80).
struct SseLineBuffer {
    buf: Vec<u8>,
}

impl SseLineBuffer {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line);
            lines.push(text.trim_end_matches(['\n', '\r']).to_string());
        }
        lines
    }
}

/// Parses one SSE line from an OpenAI-compatible streaming response.
fn parse_openai_sse_line(line: &str) -> SseEvent {
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return SseEvent::Ignore;
    };

    if data == "[DONE]" {
        return SseEvent::Done;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseEvent::Ignore;
    };

    if let Some(err) = value.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("Unknown streaming error");
        return SseEvent::Error(msg.to_string());
    }

    match value
        .pointer("/choices/0/delta/content")
        .and_then(|c| c.as_str())
    {
        Some(content) if !content.is_empty() => SseEvent::Delta(content.to_string()),
        _ => SseEvent::Ignore,
    }
}

/// Parses one SSE line from an Anthropic (Claude) streaming response.
fn parse_claude_sse_line(line: &str) -> SseEvent {
    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
        return SseEvent::Ignore;
    };

    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return SseEvent::Ignore;
    };

    match value.get("type").and_then(|t| t.as_str()) {
        Some("content_block_delta") => match value
            .pointer("/delta/text")
            .and_then(|t| t.as_str())
        {
            Some(text) if !text.is_empty() => SseEvent::Delta(text.to_string()),
            _ => SseEvent::Ignore,
        },
        Some("message_stop") => SseEvent::Done,
        Some("error") => {
            let msg = value
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown streaming error");
            SseEvent::Error(msg.to_string())
        }
        // message_start, content_block_start, content_block_stop, ping, etc.
        _ => SseEvent::Ignore,
    }
}

/// Claude requires strictly alternating user/assistant roles starting with
/// "user"; merge consecutive same-role turns defensively.
fn merge_consecutive_roles(messages: &[ChatTurn]) -> Vec<ChatTurn> {
    let mut merged: Vec<ChatTurn> = Vec::with_capacity(messages.len());
    for msg in messages {
        match merged.last_mut() {
            Some(last) if last.role == msg.role => {
                last.content.push_str("\n\n");
                last.content.push_str(&msg.content);
            }
            _ => merged.push(msg.clone()),
        }
    }
    merged
}

/// Streams a chat completion from the configured provider, invoking `on_delta`
/// for each text fragment as it arrives. Returns the full accumulated text.
///
/// `messages` is the prior chat history plus the new user message (the system
/// prompt is passed separately). The BuiltInAI sidecar cannot stream, so it
/// responds with a single delta containing the complete text.
#[allow(clippy::too_many_arguments)]
pub async fn stream_chat_completion<F>(
    client: &Client,
    provider: &LLMProvider,
    model_name: &str,
    api_key: &str,
    system_prompt: &str,
    messages: &[ChatTurn],
    ollama_endpoint: Option<&str>,
    custom_openai_endpoint: Option<&str>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    app_data_dir: Option<&PathBuf>,
    cancellation_token: &CancellationToken,
    mut on_delta: F,
) -> Result<String, String>
where
    F: FnMut(&str),
{
    if cancellation_token.is_cancelled() {
        return Err("Chat was cancelled".to_string());
    }

    // BuiltInAI sidecar has no HTTP streaming API: flatten the conversation
    // into a single prompt and emit the complete response as one delta.
    if provider == &LLMProvider::BuiltInAI {
        let app_data_dir = app_data_dir
            .ok_or_else(|| "app_data_dir is required for BuiltInAI provider".to_string())?;

        let mut user_prompt = String::new();
        for msg in messages {
            let label = if msg.role == "assistant" { "Assistant" } else { "User" };
            user_prompt.push_str(&format!("{}: {}\n\n", label, msg.content));
        }

        let text = crate::summary::summary_engine::generate_with_builtin(
            app_data_dir,
            model_name,
            system_prompt,
            user_prompt.trim_end(),
            Some(cancellation_token),
        )
        .await
        .map_err(|e| e.to_string())?;

        on_delta(&text);
        return Ok(text);
    }

    // URL and header resolution mirrors summary/llm_client.rs
    let (api_url, mut headers) = match provider {
        LLMProvider::OpenAI => (
            "https://api.openai.com/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::Groq => (
            "https://api.groq.com/openai/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::OpenRouter => (
            "https://openrouter.ai/api/v1/chat/completions".to_string(),
            header::HeaderMap::new(),
        ),
        LLMProvider::Ollama => {
            let host = ollama_endpoint
                .map(|s| s.to_string())
                .unwrap_or_else(|| "http://localhost:11434".to_string());
            (
                format!("{}/v1/chat/completions", host),
                header::HeaderMap::new(),
            )
        }
        LLMProvider::CustomOpenAI => {
            let endpoint = custom_openai_endpoint
                .ok_or_else(|| "Custom OpenAI endpoint not configured".to_string())?;
            (
                format!("{}/chat/completions", endpoint.trim_end_matches('/')),
                header::HeaderMap::new(),
            )
        }
        LLMProvider::Claude => {
            let mut header_map = header::HeaderMap::new();
            header_map.insert(
                "x-api-key",
                api_key
                    .parse()
                    .map_err(|_| "Invalid API key format".to_string())?,
            );
            header_map.insert(
                "anthropic-version",
                "2023-06-01"
                    .parse()
                    .map_err(|_| "Invalid anthropic version".to_string())?,
            );
            (
                "https://api.anthropic.com/v1/messages".to_string(),
                header_map,
            )
        }
        LLMProvider::BuiltInAI => {
            unreachable!("BuiltInAI is handled before this match statement")
        }
    };

    if provider != &LLMProvider::Claude {
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {}", api_key)
                .parse()
                .map_err(|_| "Invalid authorization header".to_string())?,
        );
    }
    headers.insert(
        header::CONTENT_TYPE,
        "application/json"
            .parse()
            .map_err(|_| "Invalid content type".to_string())?,
    );

    let is_claude = provider == &LLMProvider::Claude;
    let request_body = if is_claude {
        serde_json::json!(ClaudeStreamingRequest {
            model: model_name.to_string(),
            max_tokens: max_tokens.unwrap_or(CLAUDE_MAX_TOKENS),
            system: system_prompt.to_string(),
            messages: merge_consecutive_roles(messages),
            stream: true,
        })
    } else {
        let mut all_messages = Vec::with_capacity(messages.len() + 1);
        all_messages.push(ChatTurn {
            role: "system".to_string(),
            content: system_prompt.to_string(),
        });
        all_messages.extend(messages.iter().cloned());

        serde_json::json!(StreamingChatRequest {
            model: model_name.to_string(),
            messages: all_messages,
            stream: true,
            max_tokens,
            temperature,
            top_p,
        })
    };

    info!(
        "🐞 Chat stream request to {:?}: model={}",
        provider, model_name
    );

    let request_future = client
        .post(api_url)
        .headers(headers)
        .json(&request_body)
        .timeout(REQUEST_TIMEOUT_DURATION)
        .send();

    let response = tokio::select! {
        result = request_future => {
            result.map_err(|e| {
                if e.is_timeout() {
                    "Chat request timed out".to_string()
                } else {
                    format!("Failed to send chat request: {}", e)
                }
            })?
        }
        _ = cancellation_token.cancelled() => {
            return Err("Chat was cancelled".to_string());
        }
    };

    if !response.status().is_success() {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("LLM API request failed: {}", error_body));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = SseLineBuffer::new();
    let mut full_text = String::new();

    loop {
        let chunk = tokio::select! {
            chunk = stream.next() => chunk,
            _ = cancellation_token.cancelled() => {
                return Err("Chat was cancelled".to_string());
            }
        };

        let bytes = match chunk {
            Some(Ok(bytes)) => bytes,
            Some(Err(e)) => return Err(format!("Stream error: {}", e)),
            None => break, // stream closed
        };

        for line in line_buffer.push(&bytes) {
            let event = if is_claude {
                parse_claude_sse_line(&line)
            } else {
                parse_openai_sse_line(&line)
            };

            match event {
                SseEvent::Delta(text) => {
                    full_text.push_str(&text);
                    on_delta(&text);
                }
                SseEvent::Done => return Ok(full_text),
                SseEvent::Error(msg) => return Err(format!("LLM streaming error: {}", msg)),
                SseEvent::Ignore => {}
            }
        }
    }

    // Stream ended without an explicit done marker; treat accumulated text as
    // the complete response (some OpenAI-compatible servers just close).
    Ok(full_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_buffer_handles_split_lines() {
        let mut buf = SseLineBuffer::new();
        // A data line split across two network chunks
        let lines = buf.push(b"data: {\"choices\":[{\"delta\":{\"con");
        assert!(lines.is_empty());
        let lines = buf.push(b"tent\":\"hi\"}}]}\n\n");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            parse_openai_sse_line(&lines[0]),
            SseEvent::Delta("hi".to_string())
        );
        assert_eq!(parse_openai_sse_line(&lines[1]), SseEvent::Ignore);
    }

    #[test]
    fn openai_done_marker() {
        assert_eq!(parse_openai_sse_line("data: [DONE]"), SseEvent::Done);
    }

    #[test]
    fn claude_delta_and_stop() {
        assert_eq!(
            parse_claude_sse_line(
                r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#
            ),
            SseEvent::Delta("Hello".to_string())
        );
        assert_eq!(
            parse_claude_sse_line(r#"data: {"type":"message_stop"}"#),
            SseEvent::Done
        );
        assert_eq!(
            parse_claude_sse_line(r#"data: {"type":"ping"}"#),
            SseEvent::Ignore
        );
        assert_eq!(
            parse_claude_sse_line("event: content_block_delta"),
            SseEvent::Ignore
        );
    }

    #[test]
    fn merges_consecutive_same_role_messages() {
        let messages = vec![
            ChatTurn { role: "user".into(), content: "a".into() },
            ChatTurn { role: "user".into(), content: "b".into() },
            ChatTurn { role: "assistant".into(), content: "c".into() },
        ];
        let merged = merge_consecutive_roles(&messages);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].content, "a\n\nb");
    }
}
