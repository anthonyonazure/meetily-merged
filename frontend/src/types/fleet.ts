/** Managed (fleet) configuration. Mirrors src-tauri/src/fleet. */

export type ConsentLevel = 'self_only' | 'notify' | 'affirmative' | 'per_speaker';
export type ConsentEnforcement = 'flag_only' | 'strict';

/** Keys an administrator may name in `locked`. */
export type ManagedKey =
  | 'default_privacy_profile'
  | 'consent_level_floor'
  | 'consent_enforcement'
  | 'blocked_title_keywords'
  | 'blocked_domains'
  | 'retention_days'
  | 'allowed_transcription_providers'
  | 'allowed_llm_providers'
  | 'updates_enabled';

export interface ManagedConfig {
  default_privacy_profile: string | null;
  consent_level_floor: ConsentLevel | null;
  consent_enforcement: ConsentEnforcement | null;
  blocked_title_keywords: string[] | null;
  blocked_domains: string[] | null;
  retention_days: number | null;
  allowed_transcription_providers: string[] | null;
  allowed_llm_providers: string[] | null;
  updates_enabled: boolean | null;
  locked: ManagedKey[];
  warnings: string[];
}

export interface ManagedState {
  config: ManagedConfig;
  path: string;
  found: boolean;
  error: string | null;
  lockable_keys: ManagedKey[];
}
