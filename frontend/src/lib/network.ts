/** Thin typed wrappers over the network transparency commands. */

import { invoke } from '@tauri-apps/api/core';
import {
  ExpectedHostsReport,
  MeetingNetworkReport,
  NetworkActivity,
  NetworkExportResult,
} from '@/types/network';

export function getNetworkActivity(): Promise<NetworkActivity> {
  return invoke<NetworkActivity>('network_events_recent');
}

export function getMeetingNetworkReport(meetingId: string): Promise<MeetingNetworkReport> {
  return invoke<MeetingNetworkReport>('network_events_for_meeting', { meetingId });
}

export function getExpectedHosts(): Promise<ExpectedHostsReport> {
  return invoke<ExpectedHostsReport>('network_expected_hosts');
}

export function exportNetworkLog(args?: {
  from?: string | null;
  to?: string | null;
}): Promise<NetworkExportResult> {
  return invoke<NetworkExportResult>('network_events_export', {
    from: args?.from ?? null,
    to: args?.to ?? null,
  });
}

/** What each purpose means, for a reader checking a privacy claim. */
export const PURPOSE_LABEL: Record<string, string> = {
  model_download: 'Model download',
  llm_call: 'Language model',
  transcription: 'Transcription',
  graph_api: 'Microsoft 365',
  share_webhook: 'Share to chat',
  update_check: 'Update check',
  provider_metadata: 'Provider check',
  license_check: 'Licence check',
};

/** Bytes as something a person can read. */
export function formatBytes(bytes: number): string {
  if (bytes <= 0) return '—';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function formatTimestamp(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return date.toLocaleString();
}
