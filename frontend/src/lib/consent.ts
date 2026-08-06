/**
 * Recording Consent client: the Tauri command wrappers plus the plain-English
 * copy the UI shows.
 *
 * The copy here describes mechanics only — what the app does, who it logs.
 * It deliberately makes no claim about what any of it means legally, because
 * that depends on where everyone in the call happens to be sitting.
 */

import { invoke } from '@tauri-apps/api/core';
import type {
  AttendeeDecision,
  ConsentEvent,
  ConsentExportResult,
  ConsentLevel,
  ConsentMethod,
  ConsentPlan,
  ConsentRedactionState,
  ConsentSession,
  ConsentSettings,
  SpeakerConsentStatus,
} from '@/types/consent';

/** Prefixes the Rust gate uses so the UI can react without parsing prose. */
export const CONSENT_REQUIRED = 'CONSENT_REQUIRED';
export const CONSENT_BLOCKED = 'CONSENT_BLOCKED';

export function isConsentError(error: unknown): 'required' | 'blocked' | null {
  const text = error instanceof Error ? error.message : String(error ?? '');
  if (text.includes(CONSENT_BLOCKED)) return 'blocked';
  if (text.includes(CONSENT_REQUIRED)) return 'required';
  return null;
}

interface LevelCopy {
  label: string;
  /** One line, in the operator's terms, describing what happens. */
  summary: string;
  /** What lands in the log at this level. */
  logs: string;
}

export const LEVEL_COPY: Record<ConsentLevel, LevelCopy> = {
  self_only: {
    label: 'Just me',
    summary: 'Records with no announcement and no prompts. Nobody else in the meeting is told anything by the app.',
    logs: 'Logs one line: you consented, no other parties were notified.',
  },
  notify: {
    label: 'Tell the room',
    summary: 'Before recording starts you get a disclaimer to paste into the meeting chat, and optionally an announcement played out loud through your speakers.',
    logs: 'Logs that notice was given, how, and when.',
  },
  affirmative: {
    label: 'Tick off each attendee',
    summary: 'Recording will not start until you have ticked each named attendee as told and not objecting. Names come from your calendar when it is connected, otherwise you type them.',
    logs: 'Logs every attendee with a confirmed or declined state.',
  },
  per_speaker: {
    label: 'Confirm every speaker',
    summary: 'Recording starts right away. As each distinct speaker is identified you confirm them one at a time; speakers you have not confirmed are marked.',
    logs: 'Logs each per-speaker decision as you make it.',
  },
};

export const ENFORCEMENT_COPY = {
  flag_only: {
    label: 'Mark them',
    summary: 'The transcript keeps everything and marks the unconfirmed speakers.',
  },
  strict: {
    label: 'Withhold their words',
    summary: 'An unconfirmed speaker\'s text is held back from summaries, agents, chat, and exports. The stored transcript is untouched, so confirming later restores it.',
  },
} as const;

export const METHOD_COPY: Record<ConsentMethod, string> = {
  chat_paste: 'Pasted in meeting chat',
  spoken_announcement: 'Spoken announcement',
  verbal: 'Said out loud',
  in_person: 'In person',
  other: 'Other',
};

export const EVENT_COPY: Record<string, string> = {
  self: 'You consented for yourself',
  notice_given: 'Notice given',
  attendee_confirmed: 'Attendee confirmed',
  attendee_declined: 'Attendee declined',
  speaker_confirmed: 'Speaker confirmed',
  speaker_declined: 'Speaker declined',
  recording_blocked: 'Recording blocked',
  level_overridden: 'Level overridden',
};

export function eventLabel(eventType: string): string {
  return EVENT_COPY[eventType] ?? eventType;
}

/**
 * Safe accessor for the level copy. The backend only ever sends one of the four
 * levels, but this renders inside the live recording indicator, and an unknown
 * string there should degrade rather than blank the page.
 */
export function levelCopy(level: string): LevelCopy {
  return (
    LEVEL_COPY[level as ConsentLevel] ?? {
      label: level,
      summary: 'Consent level in force for this recording.',
      logs: '',
    }
  );
}

export const CONSENT_LEVELS: ConsentLevel[] = [
  'self_only',
  'notify',
  'affirmative',
  'per_speaker',
];

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export function getConsentSettings(): Promise<ConsentSettings> {
  return invoke<ConsentSettings>('consent_get_settings');
}

export function saveConsentSettings(
  settings: Omit<ConsentSettings, 'spoken_announcement_supported'>,
): Promise<ConsentSettings> {
  return invoke<ConsentSettings>('consent_save_settings', { input: settings });
}

export function prepareRecording(
  meetingTitle: string,
  levelOverride?: ConsentLevel | null,
  attendees?: string[],
): Promise<ConsentPlan> {
  return invoke<ConsentPlan>('consent_prepare_recording', {
    meetingTitle,
    levelOverride: levelOverride ?? null,
    attendees: attendees ?? null,
  });
}

export function grantClearance(args: {
  sessionId: string;
  meetingTitle: string;
  level: ConsentLevel;
  attendees?: AttendeeDecision[];
  noticeMethod?: ConsentMethod | null;
  overrideBlock?: boolean;
  overrideReason?: string | null;
}): Promise<void> {
  return invoke('consent_grant_clearance', {
    sessionId: args.sessionId,
    meetingTitle: args.meetingTitle,
    level: args.level,
    attendees: args.attendees ?? null,
    noticeMethod: args.noticeMethod ?? null,
    overrideBlock: args.overrideBlock ?? false,
    overrideReason: args.overrideReason ?? null,
  });
}

export function activeConsentSession(): Promise<ConsentSession | null> {
  return invoke<ConsentSession | null>('consent_active_session');
}

export function bindConsentToMeeting(meetingId: string): Promise<boolean> {
  return invoke<boolean>('consent_bind_meeting', { meetingId });
}

export function recordConsentEvent(args: {
  eventType: string;
  meetingId?: string | null;
  level?: ConsentLevel | null;
  subject?: string | null;
  method?: ConsentMethod | null;
  detail?: string | null;
}): Promise<ConsentEvent> {
  return invoke<ConsentEvent>('consent_record_event', {
    eventType: args.eventType,
    meetingId: args.meetingId ?? null,
    level: args.level ?? null,
    subject: args.subject ?? null,
    method: args.method ?? null,
    detail: args.detail ?? null,
  });
}

export function consentLogForMeeting(meetingId: string): Promise<ConsentEvent[]> {
  return invoke<ConsentEvent[]>('consent_log_for_meeting', { meetingId });
}

export function consentSpeakersForMeeting(
  meetingId: string,
): Promise<SpeakerConsentStatus[]> {
  return invoke<SpeakerConsentStatus[]>('consent_speakers_for_meeting', { meetingId });
}

export function consentRedactionState(
  meetingId: string,
): Promise<ConsentRedactionState> {
  return invoke<ConsentRedactionState>('consent_redaction_state', { meetingId });
}

export function exportConsentLog(args: {
  from: string;
  to: string;
  meetingId?: string | null;
  clientId?: string | null;
}): Promise<ConsentExportResult> {
  return invoke<ConsentExportResult>('consent_log_export', {
    from: args.from,
    to: args.to,
    meetingId: args.meetingId ?? null,
    clientId: args.clientId ?? null,
  });
}

export function speakAnnouncement(text?: string): Promise<void> {
  return invoke('consent_speak_announcement', { text: text ?? null });
}

export function prefillAttendees(): Promise<string[]> {
  return invoke<string[]>('consent_prefill_attendees');
}

/**
 * Removes the text of unconsented speakers from transcript rows before they are
 * handed to the summary LLM. The Rust side does the same for chat, agents, and
 * exports; the summary payload is assembled here, so the filter has to be here
 * too.
 */
export const WITHHELD_MARKER = '[withheld: speaker consent not confirmed]';

export function withholdUnconsented<T extends { speaker?: string | null }>(
  rows: T[],
  redaction: ConsentRedactionState | null,
): { rows: T[]; withheld: number } {
  if (!redaction?.strict || redaction.unconsented_speakers.length === 0) {
    return { rows, withheld: 0 };
  }
  const blocked = new Set(
    redaction.unconsented_speakers.map(s => s.trim().toLowerCase()),
  );
  let withheld = 0;
  const filtered = rows.filter(row => {
    const label = row.speaker?.trim().toLowerCase();
    if (label && blocked.has(label)) {
      withheld += 1;
      return false;
    }
    return true;
  });
  return { rows: filtered, withheld };
}
