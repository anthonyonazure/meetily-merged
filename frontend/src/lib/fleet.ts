/** Thin typed wrappers over the managed-configuration commands. */

import { invoke } from '@tauri-apps/api/core';
import { ManagedKey, ManagedState } from '@/types/fleet';

export function getManagedConfig(): Promise<ManagedState> {
  return invoke<ManagedState>('managed_config_get');
}

export function reloadManagedConfig(): Promise<ManagedState> {
  return invoke<ManagedState>('managed_config_reload');
}

/** True when the organisation set this key and the local user cannot change it. */
export function isLocked(state: ManagedState | null, key: ManagedKey): boolean {
  return state?.config.locked.includes(key) ?? false;
}

/** True when the organisation set this key at all, locked or not. */
export function isManaged(state: ManagedState | null, key: ManagedKey): boolean {
  if (!state) return false;
  const config = state.config;
  switch (key) {
    case 'default_privacy_profile':
      return config.default_privacy_profile !== null;
    case 'consent_level_floor':
      return config.consent_level_floor !== null;
    case 'consent_enforcement':
      return config.consent_enforcement !== null;
    case 'blocked_title_keywords':
      return config.blocked_title_keywords !== null;
    case 'blocked_domains':
      return config.blocked_domains !== null;
    case 'retention_days':
      return config.retention_days !== null;
    case 'allowed_transcription_providers':
      return config.allowed_transcription_providers !== null;
    case 'allowed_llm_providers':
      return config.allowed_llm_providers !== null;
    case 'updates_enabled':
      return config.updates_enabled !== null;
    default:
      return false;
  }
}

/** What each managed key controls, in plain English. */
export const MANAGED_KEY_LABEL: Record<ManagedKey, string> = {
  default_privacy_profile: 'Default privacy profile',
  consent_level_floor: 'Minimum consent level',
  consent_enforcement: 'Per-speaker enforcement',
  blocked_title_keywords: 'Blocked meeting-title words',
  blocked_domains: 'Blocked attendee domains',
  retention_days: 'Longest time anything is kept',
  allowed_transcription_providers: 'Allowed transcription providers',
  allowed_llm_providers: 'Allowed model providers',
  updates_enabled: 'Update checks',
};
