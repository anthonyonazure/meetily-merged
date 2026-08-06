import { useCallback, useRef, useState } from 'react';
import { toast } from 'sonner';
import { grantClearance, isConsentError, prepareRecording } from '@/lib/consent';
import type { ConsentLevel, ConsentPlan } from '@/types/consent';

interface UseConsentGateReturn {
  /** Non-null while the pre-record sheet should be shown. */
  consentPlan: ConsentPlan | null;
  /**
   * Resolves true when the recording may proceed. Returns immediately at levels
   * that need nothing from the operator, so the low-friction path stays
   * low-friction.
   */
  ensureConsent: (meetingTitle: string) => Promise<boolean>;
  /** Wired to the sheet's confirm action. */
  resolveConsent: () => void;
  /** Wired to the sheet's cancel action. */
  rejectConsent: () => void;
  /** Turns a start-command rejection from the Rust gate into a visible reason. */
  reportStartError: (error: unknown) => boolean;
  /** Level for the next recording only. Null means "use the saved default". */
  levelOverride: ConsentLevel | null;
  setLevelOverride: (level: ConsentLevel | null) => void;
}

/**
 * Frontend half of the consent gate.
 *
 * The Rust gate in `consent/gate.rs` is the enforcement; this hook exists so the
 * operator gets a sheet to act in rather than an error, and so every start path
 * in `useRecordingStart` shares one implementation instead of three.
 */
export function useConsentGate(): UseConsentGateReturn {
  const [consentPlan, setConsentPlan] = useState<ConsentPlan | null>(null);
  const [levelOverride, setLevelOverride] = useState<ConsentLevel | null>(null);
  const pendingRef = useRef<((cleared: boolean) => void) | null>(null);

  const settle = useCallback((cleared: boolean) => {
    const resolve = pendingRef.current;
    pendingRef.current = null;
    setConsentPlan(null);
    // An override is good for one recording, cleared or abandoned.
    if (cleared) setLevelOverride(null);
    resolve?.(cleared);
  }, []);

  const ensureConsent = useCallback(async (meetingTitle: string): Promise<boolean> => {
    let plan: ConsentPlan;
    try {
      plan = await prepareRecording(meetingTitle, levelOverride);
    } catch (error) {
      // The Rust gate still runs on the start command, so a failed plan lookup
      // means "no sheet", not "no consent check".
      console.error('[Consent] could not prepare the recording:', error);
      return true;
    }

    if (!plan.requires_sheet) {
      // Nothing to ask, but an override still has to be recorded and parked, or
      // the Rust gate would fall back to the saved default and the operator's
      // choice for this meeting would quietly evaporate.
      if (levelOverride) {
        try {
          await grantClearance({
            sessionId: plan.session_id,
            meetingTitle: plan.meeting_title,
            level: plan.level,
          });
        } catch (error) {
          console.error('[Consent] could not record the per-meeting level:', error);
        }
      }
      setLevelOverride(null);
      return true;
    }

    // Any sheet already open belongs to an abandoned attempt.
    settle(false);
    setConsentPlan(plan);
    return new Promise<boolean>(resolve => {
      pendingRef.current = resolve;
    });
  }, [settle, levelOverride]);

  const reportStartError = useCallback((error: unknown): boolean => {
    const kind = isConsentError(error);
    if (!kind) return false;
    const message = error instanceof Error ? error.message : String(error);
    const detail = message.split(': ').slice(1).join(': ') || message;
    if (kind === 'blocked') {
      toast.error('Recording blocked', { description: detail });
    } else {
      toast.error('Consent needed before recording', { description: detail });
    }
    return true;
  }, []);

  return {
    consentPlan,
    ensureConsent,
    resolveConsent: useCallback(() => settle(true), [settle]),
    rejectConsent: useCallback(() => settle(false), [settle]),
    reportStartError,
    levelOverride,
    setLevelOverride,
  };
}
