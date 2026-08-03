//! Tauri commands for chat with meetings.

use crate::chat::service;
use crate::database::models::ChatMessageRecord;
use crate::database::repositories::chat::ChatMessagesRepository;
use crate::state::AppState;
use log::info as log_info;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

/// Longest accepted question, to keep prompts (and the DB) bounded.
const MAX_QUESTION_CHARS: usize = 4_000;

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatSendResult {
    /// Id of the stored user message. The answer arrives later via the
    /// `chat-response` event; `chat_history` is the poll fallback.
    pub message_id: String,
}

/// Sends a question about one meeting (`meeting_id` set) or across recent
/// meetings (`meeting_id` null). Stores the user message, spawns the LLM
/// request in the background, and returns immediately.
#[tauri::command]
pub async fn chat_send<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
    message: String,
    model_provider: String,
    model_name: String,
) -> Result<ChatSendResult, String> {
    let question = message.trim().to_string();
    if question.is_empty() {
        return Err("Message is empty".to_string());
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(format!(
            "Message is too long (over {} characters)",
            MAX_QUESTION_CHARS
        ));
    }

    log_info!(
        "chat_send called: scope={}, provider={}",
        meeting_id.as_deref().unwrap_or("all-meetings"),
        model_provider
    );

    let pool = state.db_manager.pool().clone();
    let record = ChatMessagesRepository::insert(&pool, meeting_id.as_deref(), "user", &question)
        .await
        .map_err(|e| format!("Failed to store chat message: {}", e))?;

    let app_data_dir = app.path().app_data_dir().ok();
    tauri::async_runtime::spawn(service::execute_chat_request(
        app.clone(),
        pool,
        meeting_id,
        question,
        model_provider,
        model_name,
        app_data_dir,
    ));

    Ok(ChatSendResult {
        message_id: record.id,
    })
}

/// Returns the chat history for a scope, oldest first.
#[tauri::command]
pub async fn chat_history(
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
) -> Result<Vec<ChatMessageRecord>, String> {
    ChatMessagesRepository::history(state.db_manager.pool(), meeting_id.as_deref())
        .await
        .map_err(|e| format!("Failed to load chat history: {}", e))
}

/// Clears the chat history for a scope; returns the number of removed messages.
#[tauri::command]
pub async fn chat_clear(
    state: tauri::State<'_, AppState>,
    meeting_id: Option<String>,
) -> Result<u64, String> {
    ChatMessagesRepository::clear(state.db_manager.pool(), meeting_id.as_deref())
        .await
        .map_err(|e| format!("Failed to clear chat history: {}", e))
}
