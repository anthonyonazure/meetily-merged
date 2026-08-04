// Types for the Client Memory feature (mirrors the Rust structs in
// src-tauri/src/clients and src-tauri/src/database/models.rs).

export interface Client {
  id: string;
  name: string;
  domain: string | null;
  notes: string;
  created_at: string;
}

export interface ClientWithCounts extends Client {
  meeting_count: number;
  open_commitments: number;
}

export interface ClientSuggestion {
  client_id: string;
  client_name: string;
  reason: string;
}

export type MemoryFactKind = 'commitment' | 'decision' | 'figure' | 'note';

// 'na' is used for non-commitment facts, which have no lifecycle.
export type MemoryFactStatus = 'open' | 'done' | 'dismissed' | 'na';

export interface MemoryFact {
  id: string;
  meeting_id: string;
  client_id: string | null;
  agent_run_id: string | null;
  kind: MemoryFactKind;
  subject: string;
  detail: string;
  owner: string | null;
  due_hint: string | null;
  amount: string | null;
  status: MemoryFactStatus;
  created_at: string;
  updated_at: string;
}

export interface MemoryFactWithMeeting extends MemoryFact {
  meeting_title: string;
  meeting_created_at: string;
}

export interface TimelineMeeting {
  id: string;
  title: string;
  created_at: string;
}

export interface ClientTimeline {
  client: Client;
  meetings: TimelineMeeting[];
  facts: MemoryFactWithMeeting[];
}
