use crate::chat::prompt::build_chat_system_prompt;
use crate::chat::stream::{stream_chat_completion, ChatTurn};
use crate::database::repositories::chat::{ChatMessageModel, ChatMessagesRepository};
use crate::database::repositories::setting::SettingsRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::ollama::metadata::ModelMetadataCache;
use crate::state::AppState;
use crate::summary::llm_client::LLMProvider;
use log::{error as log_error, info as log_info, warn as log_warn};
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio_util::sync::CancellationToken;

/// Max chat history turns sent to the LLM (leaves room for the transcript).
const MAX_HISTORY_TURNS: usize = 20;

/// Transcript character budget for cloud providers with large contexts.
const CLOUD_TRANSCRIPT_CHAR_BUDGET: usize = 120_000;

/// Fallback transcript character budget for local models of unknown context.
const LOCAL_FALLBACK_CHAR_BUDGET: usize = 12_000;

// Model metadata cache for Ollama context-size lookups (5 minute TTL)
static METADATA_CACHE: Lazy<ModelMetadataCache> =
    Lazy::new(|| ModelMetadataCache::new(Duration::from_secs(300)));

// One active chat stream per meeting, keyed by meeting_id
static CHAT_CANCELLATION_REGISTRY: Lazy<Arc<Mutex<HashMap<String, CancellationToken>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

// Shared HTTP client for streaming requests
static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(reqwest::Client::new);

/// Streaming event emitted to the frontend as `chat-stream-chunk`.
#[derive(Debug, Clone, Serialize)]
pub struct ChatStreamChunk {
    pub meeting_id: String,
    pub message_id: String,
    pub delta: String,
    pub done: bool,
    pub error: Option<String>,
}

/// Registers a cancellation token for a meeting's chat stream.
/// Returns None if a stream is already active for the meeting.
fn try_register_chat_token(meeting_id: &str) -> Option<CancellationToken> {
    let mut registry = CHAT_CANCELLATION_REGISTRY.lock().ok()?;
    if registry.contains_key(meeting_id) {
        return None;
    }
    let token = CancellationToken::new();
    registry.insert(meeting_id.to_string(), token.clone());
    Some(token)
}

fn cleanup_chat_token(meeting_id: &str) {
    if let Ok(mut registry) = CHAT_CANCELLATION_REGISTRY.lock() {
        registry.remove(meeting_id);
    }
}

/// Resolves the transcript character budget based on the provider's context size.
async fn resolve_transcript_char_budget(
    provider: &LLMProvider,
    model_name: &str,
    ollama_endpoint: Option<&str>,
) -> usize {
    match provider {
        LLMProvider::Ollama => match METADATA_CACHE.get_or_fetch(model_name, ollama_endpoint).await
        {
            Ok(metadata) => {
                // Reserve ~1000 tokens for the question, history, and answer;
                // ~4 chars per token.
                metadata.context_size.saturating_sub(1000).saturating_mul(4)
            }
            Err(e) => {
                log_warn!(
                    "Failed to fetch context size for {}: {}. Using fallback budget",
                    model_name,
                    e
                );
                LOCAL_FALLBACK_CHAR_BUDGET
            }
        },
        LLMProvider::BuiltInAI => {
            use crate::summary::summary_engine::models;
            match models::get_model_by_name(model_name) {
                Some(model_def) => {
                    (model_def.context_size as usize).saturating_sub(1000).saturating_mul(4)
                }
                None => LOCAL_FALLBACK_CHAR_BUDGET,
            }
        }
        _ => CLOUD_TRANSCRIPT_CHAR_BUDGET,
    }
}

/// Sends a user message and streams the assistant response.
///
/// `transcript_text` carries the in-memory live transcript during recording.
/// When empty, the transcript is loaded from the database (saved meetings).
/// Returns `{ user_message_id, assistant_message_id }` immediately; the
/// response streams via `chat-stream-chunk` events.
#[tauri::command]
pub async fn api_send_chat_message<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    message: String,
    transcript_text: String,
) -> Result<serde_json::Value, String> {
    log_info!("api_send_chat_message called for meeting_id: {}", meeting_id);

    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }

    let pool = state.db_manager.pool().clone();

    // Resolve transcript context: live transcript from the frontend, or the
    // saved transcript from the database for completed meetings.
    let transcript_text = if transcript_text.trim().is_empty() {
        TranscriptsRepository::get_full_transcript_text(&pool, &meeting_id)
            .await
            .map_err(|e| format!("Failed to load transcript: {}", e))?
    } else {
        transcript_text
    };

    if transcript_text.trim().is_empty() {
        return Err("No transcript available for this meeting yet".to_string());
    }

    // Resolve provider configuration (same pattern as summary generation)
    let model_config = SettingsRepository::get_model_config(&pool)
        .await
        .map_err(|e| format!("Failed to load model config: {}", e))?
        .ok_or_else(|| "No AI model configured. Choose a model in settings.".to_string())?;

    let model_provider = model_config.provider.clone();
    let model_name = model_config.model.clone();
    let provider = LLMProvider::from_str(&model_provider)?;

    let api_key = if provider == LLMProvider::Ollama
        || provider == LLMProvider::BuiltInAI
        || provider == LLMProvider::CustomOpenAI
    {
        String::new()
    } else {
        match SettingsRepository::get_api_key(&pool, &model_provider).await {
            Ok(Some(key)) if !key.is_empty() => key,
            Ok(_) => return Err(format!("API key not found for {}", model_provider)),
            Err(e) => {
                return Err(format!(
                    "Failed to retrieve API key for {}: {}",
                    model_provider, e
                ))
            }
        }
    };

    let ollama_endpoint = if provider == LLMProvider::Ollama {
        model_config.ollama_endpoint.clone()
    } else {
        None
    };

    let (custom_openai_endpoint, custom_openai_api_key, max_tokens, temperature, top_p) =
        if provider == LLMProvider::CustomOpenAI {
            match SettingsRepository::get_custom_openai_config(&pool).await {
                Ok(Some(config)) => (
                    Some(config.endpoint),
                    config.api_key,
                    config.max_tokens.map(|t| t as u32),
                    config.temperature,
                    config.top_p,
                ),
                Ok(None) => {
                    return Err(
                        "Custom OpenAI provider selected but no configuration found".to_string()
                    )
                }
                Err(e) => return Err(format!("Failed to retrieve custom OpenAI config: {}", e)),
            }
        } else {
            (None, None, None, None, None)
        };

    let final_api_key = if provider == LLMProvider::CustomOpenAI {
        custom_openai_api_key.unwrap_or_default()
    } else {
        api_key
    };

    let app_data_dir = app.path().app_data_dir().ok();

    // One active stream per meeting
    let cancellation_token = try_register_chat_token(&meeting_id)
        .ok_or_else(|| "A chat response is already in progress for this meeting".to_string())?;

    // Persist the user message, load prior history, and create the assistant
    // row up front so its id is stable for the event stream.
    let user_message =
        match ChatMessagesRepository::insert_message(&pool, &meeting_id, "user", &message).await {
            Ok(msg) => msg,
            Err(e) => {
                cleanup_chat_token(&meeting_id);
                return Err(format!("Failed to save chat message: {}", e));
            }
        };

    let history: Vec<ChatTurn> =
        match ChatMessagesRepository::get_messages_for_meeting(&pool, &meeting_id).await {
            Ok(messages) => {
                let turns: Vec<ChatTurn> = messages
                    .into_iter()
                    .filter(|m| !m.content.trim().is_empty())
                    .map(|m| ChatTurn {
                        role: m.role,
                        content: m.content,
                    })
                    .collect();
                let skip = turns.len().saturating_sub(MAX_HISTORY_TURNS);
                turns.into_iter().skip(skip).collect()
            }
            Err(e) => {
                cleanup_chat_token(&meeting_id);
                return Err(format!("Failed to load chat history: {}", e));
            }
        };

    let assistant_message =
        match ChatMessagesRepository::insert_message(&pool, &meeting_id, "assistant", "").await {
            Ok(msg) => msg,
            Err(e) => {
                cleanup_chat_token(&meeting_id);
                return Err(format!("Failed to create assistant message: {}", e));
            }
        };

    let response = serde_json::json!({
        "user_message_id": user_message.id,
        "assistant_message_id": assistant_message.id,
    });

    // Stream the response in the background; the command returns immediately.
    let assistant_id = assistant_message.id;
    let meeting_id_task = meeting_id.clone();
    tauri::async_runtime::spawn(async move {
        let char_budget =
            resolve_transcript_char_budget(&provider, &model_name, ollama_endpoint.as_deref())
                .await;
        let system_prompt = build_chat_system_prompt(&transcript_text, char_budget);

        let mut partial_text = String::new();
        let result = stream_chat_completion(
            &HTTP_CLIENT,
            &provider,
            &model_name,
            &final_api_key,
            &system_prompt,
            &history,
            ollama_endpoint.as_deref(),
            custom_openai_endpoint.as_deref(),
            max_tokens,
            temperature,
            top_p,
            app_data_dir.as_ref(),
            &cancellation_token,
            |delta| {
                partial_text.push_str(delta);
                let chunk = ChatStreamChunk {
                    meeting_id: meeting_id_task.clone(),
                    message_id: assistant_id.clone(),
                    delta: delta.to_string(),
                    done: false,
                    error: None,
                };
                if let Err(e) = app.emit("chat-stream-chunk", &chunk) {
                    log_error!("Failed to emit chat stream chunk: {}", e);
                }
            },
        )
        .await;

        let was_cancelled = cancellation_token.is_cancelled();
        cleanup_chat_token(&meeting_id_task);

        let (final_content, error) = match result {
            Ok(full_text) => (full_text, None),
            Err(_) if was_cancelled => {
                let content = if partial_text.is_empty() {
                    "_[response cancelled]_".to_string()
                } else {
                    format!("{}\n\n_[response cancelled]_", partial_text)
                };
                (content, Some("cancelled".to_string()))
            }
            Err(e) => {
                log_error!("Chat stream failed for {}: {}", meeting_id_task, e);
                let content = format!("⚠️ {}", e);
                (content, Some(e))
            }
        };

        if let Err(e) =
            ChatMessagesRepository::update_message_content(&pool, &assistant_id, &final_content)
                .await
        {
            log_error!("Failed to persist assistant message {}: {}", assistant_id, e);
        }

        let done_chunk = ChatStreamChunk {
            meeting_id: meeting_id_task.clone(),
            message_id: assistant_id.clone(),
            delta: String::new(),
            done: true,
            error,
        };
        if let Err(e) = app.emit("chat-stream-chunk", &done_chunk) {
            log_error!("Failed to emit chat done event: {}", e);
        }
    });

    Ok(response)
}

/// Cancels the in-progress chat stream for a meeting.
#[tauri::command]
pub async fn api_cancel_chat(meeting_id: String) -> Result<serde_json::Value, String> {
    log_info!("api_cancel_chat called for meeting_id: {}", meeting_id);

    let cancelled = if let Ok(registry) = CHAT_CANCELLATION_REGISTRY.lock() {
        if let Some(token) = registry.get(&meeting_id) {
            token.cancel();
            true
        } else {
            false
        }
    } else {
        false
    };

    Ok(serde_json::json!({
        "cancelled": cancelled,
        "meeting_id": meeting_id,
    }))
}

/// Returns the chat history for a meeting in chronological order.
#[tauri::command]
pub async fn api_get_chat_history(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<ChatMessageModel>, String> {
    ChatMessagesRepository::get_messages_for_meeting(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|e| format!("Failed to load chat history: {}", e))
}

/// Deletes all chat messages for a meeting.
#[tauri::command]
pub async fn api_clear_chat_history(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<serde_json::Value, String> {
    let deleted =
        ChatMessagesRepository::delete_messages_for_meeting(state.db_manager.pool(), &meeting_id)
            .await
            .map_err(|e| format!("Failed to clear chat history: {}", e))?;

    Ok(serde_json::json!({ "deleted": deleted }))
}

/// Re-keys chat messages from the provisional live-recording meeting id to the
/// final id assigned when the meeting was saved.
#[tauri::command]
pub async fn api_reassign_chat_history(
    state: tauri::State<'_, AppState>,
    old_meeting_id: String,
    new_meeting_id: String,
) -> Result<serde_json::Value, String> {
    let updated = ChatMessagesRepository::reassign_meeting_id(
        state.db_manager.pool(),
        &old_meeting_id,
        &new_meeting_id,
    )
    .await
    .map_err(|e| format!("Failed to reassign chat history: {}", e))?;

    Ok(serde_json::json!({ "updated": updated }))
}
