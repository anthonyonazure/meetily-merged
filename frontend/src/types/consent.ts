/**
 * Recording Consent types, mirroring the Rust payloads in
 * `src-tauri/src/consent/`.
 */

export type ConsentLevel = 'self_only' | 'notify' | 'affirmative' | 'per_speaker';

export type EnforcementMode = 'flag_only' | 'strict';

export type ConsentEventType =
  | 'self'
  | 'notice_given'
  | 'attendee_confirmed'
  | 'attendee_declined'
  | 'speaker_confirmed'
  | 'speaker_declined'
  | 'recording_blocked'
  | 'level_overridden';

export type ConsentMethod =
  | 'chat_paste'
  | 'spoken_announcement'
  | 'verbal'
  | 'in_person'
  | 'other';

export type ConsentDecisionState = 'consented' | 'declined' | 'unknown';

export interface ConsentSettings {
  consent_level: ConsentLevel;
  per_speaker_enforcement: EnforcementMode;
  spoken_announcement_enabled: boolean;
  announcement_text: string;
  disclaimer_text: string;
  blocked_title_keywords: string[];
  blocked_domains: string[];
  /** False where this build has no speech path. */
  spoken_announcement_supported: boolean;
}

/** What the pre-record sheet needs, in one round trip. */
export interface ConsentPlan {
  session_id: string;
  meeting_title: string;
  level: ConsentLevel;
  enforcement: EnforcementMode;
  requires_sheet: boolean;
  /** Set when a blocking rule matched; recording is refused until overridden. */
  blocked_reason: string | null;
  disclaimer_text: string;
  announcement_text: string;
  spoken_announcement_enabled: boolean;
  spoken_announcement_supported: boolean;
  attendees: string[];
}

export interface ConsentEvent {
  id: string;
  meeting_id: string;
  level: ConsentLevel;
  event_type: ConsentEventType;
  subject: string | null;
  method: ConsentMethod | null;
  detail: string;
  created_at: string;
}

export interface AttendeeDecision {
  name: string;
  state: ConsentDecisionState;
}

export interface SpeakerConsentStatus {
  speaker: string;
  state: ConsentDecisionState;
  is_operator: boolean;
}

export interface ConsentRedactionState {
  level: ConsentLevel;
  enforcement: EnforcementMode;
  /** True when transcript text is actually being withheld. */
  strict: boolean;
  unconsented_speakers: string[];
}

/** The consent session in force for the current or most recent recording. */
export interface ConsentSession {
  session_id: string;
  meeting_title: string;
  level: ConsentLevel;
  enforcement: EnforcementMode;
  override_confirmed: boolean;
  attendees: string[];
  granted_at: string;
}

export interface ConsentExportResult {
  folder: string;
  csv_path: string;
  markdown_path: string;
  events: number;
}
