import React, { useState, useEffect, useRef } from 'react';
import { Switch } from '@/components/ui/switch';
import { FolderOpen, Keyboard, RotateCcw } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { DeviceSelection, SelectedDevices } from '@/components/DeviceSelection';
import { Button } from '@/components/ui/button';

import { toast } from 'sonner';

export interface RecordingPreferences {
  save_folder: string;
  auto_save: boolean;
  file_format: string;
  preferred_mic_device: string | null;
  preferred_system_device: string | null;
  /**
   * How long a silence must last before the transcript breaks to a new line, in ms.
   *
   * This is what decides sentence granularity. It was fixed at 400ms — longer than the
   * pause between sentences in fluent or professional speech, so several sentences
   * merged onto one line.
   */
  vad_redemption_ms: number;
}

export const VAD_REDEMPTION_MIN_MS = 100;
export const VAD_REDEMPTION_MAX_MS = 400;
export const VAD_REDEMPTION_DEFAULT_MS = 200;

interface RecordingSettingsProps {
  onSave?: (preferences: RecordingPreferences) => void;
}

interface DictationPermissionState {
  accessibility_granted: boolean;
  input_monitoring_granted: boolean;
}

const DEFAULT_DICTATION_HOTKEY = 'fn+space';
const DICTATION_HOTKEY_STORE_KEY = 'dictation_hotkey';
const MODIFIER_KEYS = new Set([
  'Shift',
  'Control',
  'Alt',
  'Meta',
  'Fn',
  'Function',
  'OS',
  'Super',
  'Hyper',
  'CapsLock',
]);

function normalizeHotkeyKey(key: string): string | null {
  if (!key) return null;
  if (key === ' ') return 'space';
  if (key === 'Spacebar') return 'space';
  if (key === 'Enter' || key === 'Return') return 'enter';
  if (key === 'Tab') return 'tab';
  if (key === 'Escape' || key === 'Esc') return 'esc';
  if (/^F([1-9]|1[0-9]|20)$/i.test(key)) return key.toLowerCase();

  if (key.length === 1) {
    return key.toLowerCase();
  }

  return null;
}

function buildHotkeyFromKeyboardEvent(event: KeyboardEvent): string | null {
  const modifiers: string[] = [];

  // Safari/WebKit may expose Fn through getModifierState('Fn')
  const hasFn =
    event.getModifierState?.('Fn') ||
    event.getModifierState?.('Function') ||
    event.getModifierState?.('fn');

  if (hasFn) modifiers.push('fn');
  if (event.ctrlKey) modifiers.push('ctrl');
  if (event.metaKey) modifiers.push('cmd');
  if (event.altKey) modifiers.push('option');
  if (event.shiftKey) modifiers.push('shift');

  if (MODIFIER_KEYS.has(event.key)) {
    return null;
  }

  const key = normalizeHotkeyKey(event.key);
  if (!key) {
    return null;
  }

  if (modifiers.length === 0) {
    return null;
  }

  return [...modifiers, key].join('+');
}

function getFallbackFnHotkey(event: KeyboardEvent): string | null {
  if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) {
    return null;
  }
  if (MODIFIER_KEYS.has(event.key)) {
    return null;
  }
  const key = normalizeHotkeyKey(event.key);
  if (!key) {
    return null;
  }
  return `fn+${key}`;
}

export function RecordingSettings({ onSave }: RecordingSettingsProps) {
  const [preferences, setPreferences] = useState<RecordingPreferences>({
    save_folder: '',
    auto_save: false,
    file_format: 'mp4',
    preferred_mic_device: null,
    preferred_system_device: null,
    vad_redemption_ms: VAD_REDEMPTION_DEFAULT_MS
  });
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [showRecordingNotification, setShowRecordingNotification] = useState(true);
  const [dictationHotkey, setDictationHotkey] = useState(DEFAULT_DICTATION_HOTKEY);
  const [isSavingDictationHotkey, setIsSavingDictationHotkey] = useState(false);
  const [isCapturingDictationHotkey, setIsCapturingDictationHotkey] = useState(false);
  const [captureHint, setCaptureHint] = useState<string | null>(null);
  const [isMacOS, setIsMacOS] = useState<boolean | null>(null);
  const [dictationPermissionState, setDictationPermissionState] = useState<DictationPermissionState | null>(null);
  const [isCheckingDictationPermissions, setIsCheckingDictationPermissions] = useState(false);
  const captureBoxRef = useRef<HTMLDivElement | null>(null);

  // Load recording preferences on component mount
  useEffect(() => {
    const loadPreferences = async () => {
      try {
        const prefs = await invoke<RecordingPreferences>('get_recording_preferences');
        setPreferences(prefs);
      } catch (error) {
        console.error('Failed to load recording preferences:', error);
        // If loading fails, get default folder path
        try {
          const defaultPath = await invoke<string>('get_default_recordings_folder_path');
          setPreferences(prev => ({ ...prev, save_folder: defaultPath }));
        } catch (defaultError) {
          console.error('Failed to get default folder path:', defaultError);
        }
      } finally {
        setLoading(false);
      }
    };

    loadPreferences();
  }, []);

  // Load recording notification preference
  useEffect(() => {
    const loadNotificationPref = async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const show = await store.get<boolean>('show_recording_notification') ?? true;
        setShowRecordingNotification(show);
      } catch (error) {
        console.error('Failed to load notification preference:', error);
      }
    };
    loadNotificationPref();
  }, []);

  // Load dictation hotkey preference and sync with backend listener
  useEffect(() => {
    const loadDictationHotkey = async () => {
      try {
        const backendHotkey = await invoke<string>('dictation_get_hotkey');
        let resolvedHotkey = backendHotkey || DEFAULT_DICTATION_HOTKEY;

        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const savedHotkey = await store.get<string>(DICTATION_HOTKEY_STORE_KEY);

        if (savedHotkey && savedHotkey.trim() && savedHotkey !== backendHotkey) {
          try {
            const result = await invoke<{ hotkey: string }>('dictation_set_hotkey', {
              hotkey: savedHotkey
            });
            resolvedHotkey = result.hotkey;
          } catch (syncError) {
            console.warn('Failed to sync saved dictation hotkey, using backend default:', syncError);
          }
        }

        setDictationHotkey(resolvedHotkey);
      } catch (error) {
        console.error('Failed to load dictation hotkey:', error);
      }
    };

    loadDictationHotkey();
  }, []);

  const detectIsMacOS = async (): Promise<boolean> => {
    try {
      const { platform } = await import('@tauri-apps/plugin-os');
      const mac = platform() === 'macos';
      setIsMacOS(mac);
      return mac;
    } catch (error) {
      console.error('Failed to detect platform:', error);
      setIsMacOS(false);
      return false;
    }
  };

  const loadDictationPermissionState = async (): Promise<DictationPermissionState | null> => {
    if (isMacOS !== true) {
      setDictationPermissionState(null);
      return null;
    }

    setIsCheckingDictationPermissions(true);
    try {
      const [accessibility_granted, input_monitoring_granted] = await Promise.all([
        invoke<boolean>('dictation_check_accessibility'),
        invoke<boolean>('dictation_check_input_monitoring')
      ]);
      const state = { accessibility_granted, input_monitoring_granted };
      setDictationPermissionState(state);
      return state;
    } catch (error) {
      console.error('Failed to check dictation permissions:', error);
      return null;
    } finally {
      setIsCheckingDictationPermissions(false);
    }
  };

  const requestAccessibilityPermission = async () => {
    try {
      const granted = await invoke<boolean>('dictation_request_accessibility');
      if (granted) {
        toast.success('Accessibility permission granted');
        await invoke<string>('dictation_restart_listener');
      } else {
        toast.error('Accessibility permission is required for dictation hotkey');
      }
      await loadDictationPermissionState();
    } catch (error) {
      console.error('Failed to request accessibility permission:', error);
      toast.error('Failed to request Accessibility permission');
    }
  };

  useEffect(() => {
    detectIsMacOS();
  }, []);

  useEffect(() => {
    if (isMacOS) {
      loadDictationPermissionState();
    }
  }, [isMacOS]);

  const requestInputMonitoringPermission = async () => {
    try {
      const granted = await invoke<boolean>('dictation_request_input_monitoring');
      if (granted) {
        toast.success('Input Monitoring permission granted');
        await invoke<string>('dictation_restart_listener');
      } else {
        toast.error('Input Monitoring permission is required for dictation hotkey');
      }
      await loadDictationPermissionState();
    } catch (error) {
      console.error('Failed to request Input Monitoring permission:', error);
      toast.error('Failed to request Input Monitoring permission');
    }
  };

  const ensureDictationPermissions = async (): Promise<boolean> => {
    const macOS = isMacOS === null ? await detectIsMacOS() : isMacOS;
    if (!macOS) {
      return true;
    }

    const state = await loadDictationPermissionState();
    if (!state) {
      return false;
    }

    if (!state.accessibility_granted) {
      await requestAccessibilityPermission();
      return false;
    }

    if (!state.input_monitoring_granted) {
      await requestInputMonitoringPermission();
      return false;
    }

    return true;
  };

  const handleAutoSaveToggle = async (enabled: boolean) => {
    const newPreferences = { ...preferences, auto_save: enabled };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  // Dragging updates local state only — the value must track the thumb without a
  // round-trip to the backend on every tick.
  const handleVadRedemptionInput = (ms: number) => {
    const clamped = Math.min(VAD_REDEMPTION_MAX_MS, Math.max(VAD_REDEMPTION_MIN_MS, ms));
    setPreferences((prev) => ({ ...prev, vad_redemption_ms: clamped }));
  };

  // Persist once, when the drag ends. Silently: a slider does not warrant a toast.
  const handleVadRedemptionCommit = async () => {
    await savePreferences(preferences, { silent: true });
  };

  const handleDeviceChange = async (devices: SelectedDevices) => {
    const newPreferences = {
      ...preferences,
      preferred_mic_device: devices.micDevice,
      preferred_system_device: devices.systemDevice
    };
    setPreferences(newPreferences);
    await savePreferences(newPreferences);
  };

  const handleOpenFolder = async () => {
    try {
      await invoke('open_recordings_folder');
    } catch (error) {
      console.error('Failed to open recordings folder:', error);
    }
  };

  const handleNotificationToggle = async (enabled: boolean) => {
    const previous = showRecordingNotification;
    setShowRecordingNotification(enabled);
    try {
      const { Store } = await import('@tauri-apps/plugin-store');
      const store = await Store.load('preferences.json');
      await store.set('show_recording_notification', enabled);
      await store.save();
      toast.success('Preference saved');
    } catch (error) {
      setShowRecordingNotification(previous);
      console.error('Failed to save notification preference:', error);
      toast.error('Failed to save preference');
    }
  };

  const persistDictationHotkey = async (hotkey: string) => {
    const { Store } = await import('@tauri-apps/plugin-store');
    const store = await Store.load('preferences.json');
    await store.set(DICTATION_HOTKEY_STORE_KEY, hotkey);
    await store.save();
  };

  const handleSaveDictationHotkey = async (hotkeyInput?: string) => {
    const hotkeyToSave = (hotkeyInput ?? dictationHotkey).trim();
    if (!hotkeyToSave) {
      toast.error('Hotkey cannot be empty');
      return;
    }

    setIsSavingDictationHotkey(true);
    try {
      const result = await invoke<{ hotkey: string }>('dictation_set_hotkey', {
        hotkey: hotkeyToSave
      });
      setDictationHotkey(result.hotkey);
      await persistDictationHotkey(result.hotkey);

      toast.success('Dictation hotkey updated', {
        description: `Current hotkey: ${result.hotkey}`
      });
    } catch (error) {
      console.error('Failed to update dictation hotkey:', error);
      toast.error('Failed to update dictation hotkey', {
        description: error instanceof Error ? error.message : String(error)
      });
    } finally {
      setIsSavingDictationHotkey(false);
    }
  };

  const handleResetDictationHotkey = async () => {
    setDictationHotkey(DEFAULT_DICTATION_HOTKEY);
    await handleSaveDictationHotkey(DEFAULT_DICTATION_HOTKEY);
  };

  useEffect(() => {
    if (isCapturingDictationHotkey) {
      captureBoxRef.current?.focus();
    }
  }, [isCapturingDictationHotkey]);

  const handleStartHotkeyCapture = async () => {
    if (isSavingDictationHotkey) {
      return;
    }

    const canCapture = await ensureDictationPermissions();
    if (!canCapture) {
      return;
    }

    setCaptureHint(null);
    setIsCapturingDictationHotkey(true);
  };

  useEffect(() => {
    if (!isCapturingDictationHotkey) {
      return;
    }

    const onCaptureKeyDown = async (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();

      if (event.key === 'Escape') {
        setIsCapturingDictationHotkey(false);
        setCaptureHint(null);
        return;
      }

      const hotkey = buildHotkeyFromKeyboardEvent(event);
      if (hotkey) {
        setCaptureHint(`Detected: ${hotkey}`);
        setIsCapturingDictationHotkey(false);
        await handleSaveDictationHotkey(hotkey);
        return;
      }

      const fnFallback = getFallbackFnHotkey(event);
      if (fnFallback) {
        setCaptureHint(`Detected with Fn fallback: ${fnFallback}`);
        setIsCapturingDictationHotkey(false);
        await handleSaveDictationHotkey(fnFallback);
        return;
      }

      setCaptureHint('Press at least one modifier plus a key, e.g. fn+space or cmd+shift+space');
    };

    window.addEventListener('keydown', onCaptureKeyDown, true);
    return () => {
      window.removeEventListener('keydown', onCaptureKeyDown, true);
    };
  }, [isCapturingDictationHotkey, handleSaveDictationHotkey]);

  const savePreferences = async (prefs: RecordingPreferences, options?: { silent?: boolean }) => {
    // `silent` saves do not flip `saving`. Disabling a control mid-drag makes the
    // browser abort the drag and drop focus, which scrolled the settings page back to
    // the top the moment you touched the slider.
    if (!options?.silent) setSaving(true);
    try {
      await invoke('set_recording_preferences', { preferences: prefs });
      onSave?.(prefs);

      if (options?.silent) return;

      // Show success toast with device details
      const micDevice = prefs.preferred_mic_device || 'Default';
      const systemDevice = prefs.preferred_system_device || 'Default';
      toast.success("Device preferences saved", {
        description: `Microphone: ${micDevice}, System Audio: ${systemDevice}`
      });
    } catch (error) {
      console.error('Failed to save recording preferences:', error);
      toast.error("Failed to save device preferences", {
        description: error instanceof Error ? error.message : String(error)
      });
    } finally {
      if (!options?.silent) setSaving(false);
    }
  };

  if (loading) {
    return (
      <div className="animate-pulse">
        <div className="h-4 bg-gray-200 rounded w-1/4 mb-4"></div>
        <div className="h-8 bg-gray-200 rounded mb-4"></div>
      </div>
    );
  }

  const missingAccessibility =
    isMacOS && dictationPermissionState && !dictationPermissionState.accessibility_granted;
  const missingInputMonitoring =
    isMacOS && dictationPermissionState && !dictationPermissionState.input_monitoring_granted;
  const showDictationPermissionNotice = Boolean(missingAccessibility || missingInputMonitoring);

  return (
    <div className="space-y-6">
      <div>
        <h3 className="text-lg font-semibold mb-4">Recording Settings</h3>
        <p className="text-sm text-gray-600 mb-6">
          Configure how your audio recordings are saved during meetings.
        </p>
      </div>

      {/* Auto Save Toggle */}
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">Save Audio Recordings</div>
          <div className="text-sm text-gray-600">
            Automatically save audio files when recording stops
          </div>
        </div>
        <Switch
          checked={preferences.auto_save}
          onCheckedChange={handleAutoSaveToggle}
          disabled={saving}
        />
      </div>

      {/* Folder Location - Only shown when auto_save is enabled */}
      {preferences.auto_save && (
        <div className="space-y-4">
          <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Save Location</div>
            <div className="text-sm text-gray-600 mb-3 break-all">
              {preferences.save_folder || 'Default folder'}
            </div>
            <button
              onClick={handleOpenFolder}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-50 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div>

          <div className="p-4 border rounded-lg bg-blue-50">
            <div className="text-sm text-blue-800">
              <strong>File Format:</strong> {preferences.file_format.toUpperCase()} files
            </div>
            <div className="text-xs text-blue-600 mt-1">
              Recordings are saved with timestamp: recording_YYYYMMDD_HHMMSS.{preferences.file_format}
            </div>
          </div>
        </div>
      )}

      {/* Info when auto_save is disabled */}
      {!preferences.auto_save && (
        <div className="p-4 border rounded-lg bg-yellow-50">
          <div className="text-sm text-yellow-800">
            Audio recording is disabled. Enable "Save Audio Recordings" to automatically save your meeting audio.
          </div>
        </div>
      )}

      {/* Recording Notification Toggle */}
      <div className="flex items-center justify-between p-4 border rounded-lg">
        <div className="flex-1">
          <div className="font-medium">Recording Start Notification</div>
          <div className="text-sm text-gray-600">
            Show reminder to inform participants when recording starts
          </div>
        </div>
        <Switch
          checked={showRecordingNotification}
          onCheckedChange={handleNotificationToggle}
        />
      </div>

      {/* Push-to-talk dictation hotkey (macOS event tap) */}
      {isMacOS !== false && (
      <div className="p-4 border rounded-lg space-y-3">
        <div className="flex items-start gap-3">
          <div className="mt-0.5">
            <Keyboard className="w-4 h-4 text-gray-600" />
          </div>
          <div className="flex-1">
            <div className="font-medium">Push-to-talk Dictation Hotkey</div>
            <div className="text-sm text-gray-600">
              Hold this hotkey to dictate into any app&apos;s text input. Example formats:
              <span className="font-medium"> fn+space</span>,
              <span className="font-medium"> ctrl+space</span>,
              <span className="font-medium"> cmd+shift+space</span>.
            </div>
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <div
            ref={captureBoxRef}
            tabIndex={isCapturingDictationHotkey ? 0 : -1}
            className={`rounded-md border px-3 py-2 text-sm transition-colors outline-none ${
              isCapturingDictationHotkey
                ? 'border-blue-500 ring-1 ring-blue-500 bg-blue-50'
                : 'border-gray-300 bg-gray-50'
            }`}
          >
            {isCapturingDictationHotkey
              ? 'Capturing... press your shortcut now'
              : `Current hotkey: ${dictationHotkey}`}
          </div>
          {captureHint && (
            <p className="text-xs text-gray-600">{captureHint}</p>
          )}
        </div>

        <div className="flex gap-2">
          <Button
            variant={isCapturingDictationHotkey ? 'secondary' : 'outline'}
            onClick={handleStartHotkeyCapture}
            disabled={isSavingDictationHotkey}
          >
            <Keyboard className="w-4 h-4" />
            {isCapturingDictationHotkey ? 'Waiting for keys...' : 'Record Shortcut'}
          </Button>
          <Button
            variant="outline"
            onClick={() => setIsCapturingDictationHotkey(false)}
            disabled={!isCapturingDictationHotkey || isSavingDictationHotkey}
          >
            Cancel
          </Button>
          <Button
            variant="outline"
            onClick={handleResetDictationHotkey}
            disabled={isSavingDictationHotkey}
          >
            <RotateCcw className="w-4 h-4" />
            Reset
          </Button>
        </div>

        {showDictationPermissionNotice && (
          <div className="rounded-md border border-red-200 bg-red-50 p-3 space-y-3">
            <p className="text-sm text-red-700">
              Dictation hotkey needs macOS permissions before it can work.
            </p>
            <div className="text-xs text-red-700 space-y-1">
              {missingAccessibility && (
                <p>Accessibility permission is missing.</p>
              )}
              {missingInputMonitoring && (
                <p>Input Monitoring permission is missing.</p>
              )}
            </div>
            <div className="flex gap-2">
              {missingAccessibility && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={requestAccessibilityPermission}
                  disabled={isCheckingDictationPermissions}
                >
                  Grant Accessibility
                </Button>
              )}
              {missingInputMonitoring && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={requestInputMonitoringPermission}
                  disabled={isCheckingDictationPermissions}
                >
                  Grant Input Monitoring
                </Button>
              )}
            </div>
          </div>
        )}
      </div>
      )}

      {/* Device Preferences */}
      <div className="space-y-4">
        <div className="border-t pt-6">
          <h4 className="text-base font-medium text-gray-900 mb-4">Default Audio Devices</h4>
          <p className="text-sm text-gray-600 mb-4">
            Set your preferred microphone and system audio devices for recording. These will be automatically selected when starting new recordings.
          </p>

          <div className="border rounded-lg p-4 bg-gray-50">
            <DeviceSelection
              selectedDevices={{
                micDevice: preferences.preferred_mic_device,
                systemDevice: preferences.preferred_system_device
              }}
              onDeviceChange={handleDeviceChange}
              disabled={saving}
            />
          </div>
        </div>
      </div>

      {/* Pause length. This is what decides when audio is sent to the model at all,
          so it sets how quickly text appears — not just where a line breaks. */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">
          Pause before transcribing
        </h3>
        <p className="text-sm text-gray-600 mb-4">
          How long a silence must last before the line ends and the audio is transcribed. It sets both where lines break and how quickly text appears. Applies to the next recording.
        </p>

        <div className="flex items-center gap-4">
          <input
            type="range"
            min={VAD_REDEMPTION_MIN_MS}
            max={VAD_REDEMPTION_MAX_MS}
            step={10}
            value={preferences.vad_redemption_ms}
            onChange={(e) => handleVadRedemptionInput(Number(e.target.value))}
            onPointerUp={handleVadRedemptionCommit}
            onKeyUp={handleVadRedemptionCommit}
            className="h-2 flex-1 cursor-pointer appearance-none rounded-lg bg-gray-200 accent-blue-600"
          />
          <span className="w-20 shrink-0 text-right text-sm font-medium tabular-nums text-gray-900">
            {preferences.vad_redemption_ms} ms
          </span>
        </div>

        <div className="mt-2 flex justify-between text-xs text-gray-400">
          <span>100 ms — faster, more splits</span>
          <span>400 ms — slower, fewer splits</span>
        </div>
      </div>
    </div>
  );
}