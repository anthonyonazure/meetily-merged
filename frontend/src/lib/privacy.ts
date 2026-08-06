/**
 * Privacy profile client: the Tauri command wrappers plus the plain-English
 * copy the UI shows.
 *
 * The copy describes mechanics only — which provider runs where, what gets
 * deleted when, what the log records. It makes no claim about what any of that
 * means legally, because that depends on where everyone in the call is sitting.
 */

import { invoke } from '@tauri-apps/api/core';
import type { ConsentLevel } from '@/types/consent';
import type {
  EffectiveProfile,
  MeetingProfileView,
  PrivacyProfile,
  PrivacyProfileInput,
  PrivacySettings,
  ProcessingMode,
  PurgeCandidate,
  RedactionPreview,
  RetentionRunResult,
  RetentionSettings,
} from '@/types/privacy';

/** Prefix the Rust enforcement points use so the UI can explain a refusal. */
export const PROFILE_BLOCKED = 'PROFILE_BLOCKED';

export function isProfileBlocked(error: unknown): boolean {
  const text = error instanceof Error ? error.message : String(error ?? '');
  return text.includes(PROFILE_BLOCKED);
}

/** Strips the machine-readable prefix so a toast reads like a sentence. */
export function profileBlockedMessage(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error ?? '');
  const marker = `${PROFILE_BLOCKED}:`;
  const at = text.indexOf(marker);
  return at === -1 ? text : text.slice(at + marker.length).trim();
}

export const MODE_COPY: Record<ProcessingMode, { label: string; summary: string }> = {
  local_only: {
    label: 'On this machine only',
    summary: 'Only providers that run locally can be used. Picking a cloud provider is refused, with the reason shown.',
  },
  cloud_allowed: {
    label: 'Cloud allowed',
    summary: 'Cloud providers can be used, which usually means better accuracy and sending content to that provider.',
  },
};

export const RETENTION_COPY = {
  keep: 'Meetings stay until someone deletes them.',
  window: (days: number) =>
    `Recordings, transcripts, summaries, and extracted facts are removed once a meeting is more than ${days} days old. The consent log is kept.`,
};

export const REDACT_COPY = {
  on: 'Before a transcript is handed to a model, exported, or shared, card numbers that pass a checksum, US SSN patterns, known API key shapes, and the value after a cue like "the password is" are replaced with a marker. The stored transcript is never changed.',
  off: 'Transcripts are handed on as they were recorded.',
  scope:
    'This looks for shapes and spoken cues, not people. It does not find names, addresses, phone numbers, or account numbers.',
};

export const SHARING_COPY = {
  on: 'The Slack, Teams, and Outlook-draft actions are available on this client\'s meetings.',
  off: 'Those three actions are turned off for this client\'s meetings. Copy and export still work.',
};

/** One line per consequence, for the profile editor and the meeting chip. */
export function profileEffects(profile: PrivacyProfile): { allows: string[]; blocks: string[] } {
  const allows: string[] = [];
  const blocks: string[] = [];

  if (profile.transcription_mode === 'local_only') {
    blocks.push('Cloud transcription (OpenAI, remote endpoint) is refused at recording start.');
    allows.push('Local transcription (Whisper, Parakeet, Qwen) runs as usual.');
  } else {
    allows.push('Cloud transcription is allowed as well as local.');
  }

  if (profile.llm_mode === 'local_only') {
    blocks.push('Cloud models (Claude, OpenAI, Groq, OpenRouter, custom endpoints) are refused for summaries, chat, and agents.');
    allows.push('Ollama and the built-in model run as usual.');
  } else {
    allows.push('Cloud models are allowed for summaries, chat, and agents.');
  }

  if (profile.allow_sharing) {
    allows.push('Slack, Teams, and Outlook-draft sharing is available.');
  } else {
    blocks.push('Slack, Teams, and Outlook-draft sharing is turned off.');
  }

  if (profile.redact_pii) {
    allows.push('Obvious secrets are masked in the copy handed to models, exports, and shares.');
  }

  if (profile.retention_days !== null) {
    blocks.push(
      `Meetings older than ${profile.retention_days} days are purged: recording files, transcript, summary, facts, and action items. The consent log stays.`,
    );
  } else {
    allows.push('Nothing is removed on a schedule.');
  }

  return { allows, blocks };
}

export const SOURCE_COPY: Record<string, string> = {
  client_tag: 'from this meeting\'s client tag',
  workspace_default: 'as the workspace default',
  none: 'no profile applies, so the app\'s global settings govern',
};

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export function listPrivacyProfiles(): Promise<PrivacyProfile[]> {
  return invoke<PrivacyProfile[]>('privacy_profiles_list');
}

export function createPrivacyProfile(input: PrivacyProfileInput): Promise<PrivacyProfile> {
  return invoke<PrivacyProfile>('privacy_profile_create', { input });
}

export function updatePrivacyProfile(
  profileId: string,
  input: PrivacyProfileInput,
): Promise<PrivacyProfile> {
  return invoke<PrivacyProfile>('privacy_profile_update', { profileId, input });
}

export function deletePrivacyProfile(profileId: string): Promise<boolean> {
  return invoke<boolean>('privacy_profile_delete', { profileId });
}

export function privacyProfileUsage(profileId: string): Promise<number> {
  return invoke<number>('privacy_profile_usage', { profileId });
}

export function getPrivacySettings(): Promise<PrivacySettings> {
  return invoke<PrivacySettings>('privacy_settings_get');
}

export function setDefaultPrivacyProfile(profileId: string | null): Promise<PrivacySettings> {
  return invoke<PrivacySettings>('privacy_settings_set_default', { profileId });
}

export function setClientPrivacyProfile(
  clientId: string,
  profileId: string | null,
): Promise<EffectiveProfile> {
  return invoke<EffectiveProfile>('client_set_privacy_profile', { clientId, profileId });
}

export function meetingPrivacyProfile(meetingId: string): Promise<MeetingProfileView> {
  return invoke<MeetingProfileView>('meeting_privacy_profile', { meetingId });
}

export function previewRedaction(text: string): Promise<RedactionPreview> {
  return invoke<RedactionPreview>('privacy_redaction_preview', { text });
}

export function retentionPreview(): Promise<PurgeCandidate[]> {
  return invoke<PurgeCandidate[]>('retention_preview');
}

export function retentionRunNow(confirm: boolean): Promise<RetentionRunResult> {
  return invoke<RetentionRunResult>('retention_run_now', { confirm });
}

export function getRetentionSettings(): Promise<RetentionSettings> {
  return invoke<RetentionSettings>('retention_settings_get');
}

export function setRetentionDryRun(dryRun: boolean, confirm: boolean): Promise<RetentionSettings> {
  return invoke<RetentionSettings>('retention_settings_set', { dryRun, confirm });
}

/** The starting point for a new profile: the most restrictive sensible one. */
export function blankProfileInput(): PrivacyProfileInput {
  return {
    name: '',
    description: '',
    transcription_mode: 'local_only',
    llm_mode: 'local_only',
    consent_level: 'notify' as ConsentLevel,
    consent_enforcement: 'flag_only',
    retention_days: null,
    redact_pii: false,
    allow_sharing: true,
  };
}

export function toProfileInput(profile: PrivacyProfile): PrivacyProfileInput {
  return {
    name: profile.name,
    description: profile.description,
    transcription_mode: profile.transcription_mode,
    llm_mode: profile.llm_mode,
    consent_level: profile.consent_level,
    consent_enforcement: profile.consent_enforcement,
    retention_days: profile.retention_days,
    redact_pii: profile.redact_pii,
    allow_sharing: profile.allow_sharing,
  };
}
