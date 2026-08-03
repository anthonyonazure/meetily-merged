// Types for the chat-with-meetings feature (mirrors the Rust structs in
// src-tauri/src/chat and src-tauri/src/database/models.rs).

export type ChatRole = 'user' | 'assistant';

export interface ChatMessageRecord {
  id: string;
  meeting_id: string | null;
  role: ChatRole;
  content: string;
  created_at: string;
}

export interface ChatSendResult {
  message_id: string;
}

export interface ChatResponsePayload {
  meeting_id: string | null;
  message: ChatMessageRecord;
  is_error: boolean;
}
