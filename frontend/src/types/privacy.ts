/**
 * Privacy profile types, mirroring the Rust payloads in `src-tauri/src/profiles/`.
 */

import type { ConsentLevel, EnforcementMode } from '@/types/consent';

export type ProcessingMode = 'local_only' | 'cloud_allowed';

/** How a profile came to apply to a meeting. */
export type ProfileSource = 'client_tag' | 'workspace_default' | 'none';

export interface PrivacyProfile {
  id: string;
  name: string;
  description: string;
  transcription_mode: ProcessingMode;
  llm_mode: ProcessingMode;
  consent_level: ConsentLevel;
  consent_enforcement: EnforcementMode;
  /** Null means meetings are kept until someone deletes them. */
  retention_days: number | null;
  redact_pii: boolean;
  allow_sharing: boolean;
  /** The three shipped profiles. Editable and duplicable, never deletable. */
  is_builtin: boolean;
  created_at: string;
  updated_at: string;
}

/** The editable fields, as the editor sends them back. */
export type PrivacyProfileInput = Pick<
  PrivacyProfile,
  | 'name'
  | 'description'
  | 'transcription_mode'
  | 'llm_mode'
  | 'consent_level'
  | 'consent_enforcement'
  | 'retention_days'
  | 'redact_pii'
  | 'allow_sharing'
>;

export interface EffectiveProfile {
  profile: PrivacyProfile | null;
  source: ProfileSource;
  client_id: string | null;
  client_name: string | null;
}

export interface MeetingProfileView extends EffectiveProfile {
  /** The same one-line description that went into the consent log. */
  summary: string;
}

export interface RetentionSettings {
  /** True while purges are logged but nothing is deleted. Ships on. */
  dry_run: boolean;
  /** When dry run was turned off. Null means the hourly sweep will not delete. */
  armed_at: string | null;
  last_run_at: string | null;
}

export interface PrivacySettings {
  /** Null means no profile governs untagged meetings. */
  default_profile_id: string | null;
  retention: RetentionSettings;
}

export interface PurgeCandidate {
  meeting_id: string;
  title: string;
  created_at: string;
  age_days: number;
  profile_name: string;
  profile_source: ProfileSource;
  retention_days: number;
  /** Negative once the window has already closed. */
  days_until_purge: number;
  client_name: string | null;
}

export interface PurgeOutcome {
  meeting_id: string;
  title: string;
  profile_name: string;
  dry_run: boolean;
  files_removed: number;
  transcripts_removed: number;
  summaries_removed: number;
  facts_removed: number;
  action_items_removed: number;
}

export interface RetentionRunResult {
  dry_run: boolean;
  examined: number;
  purged: PurgeOutcome[];
  /** Set when a real purge was asked for but the safety conditions said no. */
  refused_reason: string | null;
}

export interface RedactionReport {
  cards: number;
  ssns: number;
  keys: number;
  secrets: number;
}

export interface RedactionPreview {
  masked: string;
  report: RedactionReport;
}
