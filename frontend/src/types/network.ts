/** Network transparency. Mirrors the Rust types in src-tauri/src/network. */

export type NetworkPurpose =
  | 'model_download'
  | 'llm_call'
  | 'transcription'
  | 'graph_api'
  | 'share_webhook'
  | 'update_check'
  | 'provider_metadata'
  | 'license_check';

export interface NetworkEvent {
  id: string;
  created_at: string;
  session_id: string;
  host: string;
  url: string;
  method: string;
  purpose: string;
  outcome: string;
  bytes_out: number;
  bytes_in: number;
  meeting_id: string | null;
  profile_name: string | null;
  carried_audio: boolean;
  carried_transcript: boolean;
  detail: string;
}

export interface HostTally {
  host: string;
  requests: number;
  bytes_out: number;
  bytes_in: number;
  expected: boolean;
  on_device: boolean;
}

export interface NetworkActivity {
  session_id: string;
  session_events: NetworkEvent[];
  historical_events: NetworkEvent[];
  session_tallies: HostTally[];
  all_time_tallies: HostTally[];
  session_request_count: number;
  session_host_count: number;
  total_request_count: number;
  unexpected_hosts: string[];
  headline: string;
  caveat: string;
}

export interface MeetingNetworkReport {
  meeting_id: string;
  events: NetworkEvent[];
  audio_left_device: boolean;
  transcript_left_device: boolean;
  hosts: string[];
  verdict: string;
}

export interface ExpectedHost {
  host: string;
  purpose: NetworkPurpose;
  what_for: string;
  only_when_configured: boolean;
  on_device: boolean;
}

export interface ExpectedHostsReport {
  hosts: ExpectedHost[];
  note: string;
}

export interface NetworkExportResult {
  events: number;
  csv_path: string;
  folder: string;
}
