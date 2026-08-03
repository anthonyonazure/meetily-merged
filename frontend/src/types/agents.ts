// Types for the Meeting Agents feature (mirrors the Rust structs in
// src-tauri/src/agents and src-tauri/src/database/models.rs).

export type AgentOutputKind = 'markdown' | 'action_items';

export interface AgentInfo {
  id: string;
  name: string;
  description: string;
  output_kind: AgentOutputKind;
  enabled: boolean;
  auto_run: boolean;
}

export type AgentRunStatus = 'running' | 'completed' | 'error';

export interface AgentRun {
  id: string;
  agent_id: string;
  meeting_id: string;
  status: AgentRunStatus;
  output_md: string | null;
  error: string | null;
  created_at: string;
}

export type ActionItemStatus = 'open' | 'done';

export interface ActionItem {
  id: string;
  meeting_id: string;
  agent_run_id: string | null;
  description: string;
  owner: string | null;
  due_hint: string | null;
  status: ActionItemStatus;
  created_at: string;
  updated_at: string;
}

export interface ActionItemWithMeeting extends ActionItem {
  meeting_title: string;
}
