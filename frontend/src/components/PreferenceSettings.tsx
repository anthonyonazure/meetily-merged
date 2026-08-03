"use client"

import { useEffect, useState } from "react"
import { Switch } from "./ui/switch"
import { FolderOpen, Download } from "lucide-react"
import { invoke } from "@tauri-apps/api/core"
import { useConfig } from "@/contexts/ConfigContext"
import { toast } from "sonner"

export function PreferenceSettings() {
  const {
    storageLocations,
    isLoadingPreferences,
    loadPreferences,
  } = useConfig();

  const [showRecordingReminder, setShowRecordingReminder] = useState<boolean | null>(null);
  const [detectByRunningApps, setDetectByRunningApps] = useState<boolean>(false);
  const [exportingFormat, setExportingFormat] = useState<string | null>(null);

  const handleExport = async (format: 'markdown' | 'docx' | 'pdf', label: string) => {
    setExportingFormat(format);
    try {
      const result = await invoke<{ folder: string; exported: number; skipped: number }>(
        'export_meetings',
        { format, meetingIds: null }
      );
      toast.success(`Exported ${result.exported} meeting${result.exported === 1 ? '' : 's'} as ${label}`, {
        description: format === 'pdf'
          ? `${result.folder} (print the opened page and choose "Save as PDF")`
          : result.folder,
      });
    } catch (error) {
      // The Rust side reports a cancelled folder picker as "cancelled"; that is a user
      // action, not a failure, so it must not raise an error toast.
      if (String(error) === 'cancelled') return;
      console.error('Export failed:', error);
      toast.error('Export failed', { description: String(error) });
    } finally {
      setExportingFormat(null);
    }
  };

  // Lazy load storage-location preferences on mount (only loads if not already cached)
  useEffect(() => {
    loadPreferences();
  }, [loadPreferences]);

  // Load the in-app recording reminder preference from preferences.json.
  // Mirrors the read in `frontend/src/lib/recordingNotification.tsx` so the
  // two stay in sync — same store key, same default.
  useEffect(() => {
    (async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const current = (await store.get<boolean>('show_recording_notification')) ?? true;
        setShowRecordingReminder(current);
      } catch (error) {
        console.error('Failed to load recording reminder preference:', error);
        setShowRecordingReminder(true);
      }
    })();
  }, []);

  // Load the "detect by running apps" preference and push it to the live
  // detection service (the backend flag is not persisted across restarts).
  useEffect(() => {
    (async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const enabled = (await store.get<boolean>('detect_by_running_apps')) ?? false;
        setDetectByRunningApps(enabled);
        await invoke('set_process_detection_enabled', { enabled });
      } catch (error) {
        console.error('Failed to load process-detection preference:', error);
      }
    })();
  }, []);

  const handleDetectByRunningAppsToggle = async (enabled: boolean) => {
    const previous = detectByRunningApps;
    setDetectByRunningApps(enabled);
    try {
      await invoke('set_process_detection_enabled', { enabled });
      const { Store } = await import('@tauri-apps/plugin-store');
      const store = await Store.load('preferences.json');
      await store.set('detect_by_running_apps', enabled);
      await store.save();
    } catch (error) {
      setDetectByRunningApps(previous);
      console.error('Failed to save process-detection preference:', error);
      toast.error('Failed to save preference');
    }
  };

  const handleReminderToggle = async (enabled: boolean) => {
    const previous = showRecordingReminder;
    setShowRecordingReminder(enabled);
    try {
      const { Store } = await import('@tauri-apps/plugin-store');
      const store = await Store.load('preferences.json');
      await store.set('show_recording_notification', enabled);
      await store.save();
    } catch (error) {
      setShowRecordingReminder(previous);
      console.error('Failed to save recording reminder preference:', error);
      toast.error('Failed to save preference');
    }
  };

  const handleOpenFolder = async (folderType: 'database' | 'models' | 'recordings') => {
    try {
      switch (folderType) {
        case 'database':
          await invoke('open_database_folder');
          break;
        case 'models':
          await invoke('open_models_folder');
          break;
        case 'recordings':
          await invoke('open_recordings_folder');
          break;
      }

    } catch (error) {
      console.error(`Failed to open ${folderType} folder:`, error);
    }
  };

  // Show loading only if we're actually loading and don't have cached data
  if (isLoadingPreferences && !storageLocations && showRecordingReminder === null) {
    return <div className="max-w-2xl mx-auto p-6">Loading Preferences...</div>
  }

  const reminderEnabled = showRecordingReminder ?? true;

  return (
    <div className="space-y-6">
      {/* In-app Recording Reminder Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">In-app recording reminder</h3>
            <p className="text-sm text-gray-600">
              Show the compliance reminder toast inside the app when a recording starts.
            </p>
          </div>
          <Switch checked={reminderEnabled} onCheckedChange={handleReminderToggle} />
        </div>
      </div>

      {/* Meeting detection: process-name signal (augments mic-activity detection) */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-semibold text-gray-900 mb-2">Detect by running apps</h3>
            <p className="text-sm text-gray-600">
              Also treat a running meeting app (Zoom, Teams, Webex, Slack, Discord) as a meeting
              signal, in addition to microphone activity. May notify when an app is merely open.
            </p>
          </div>
          <Switch checked={detectByRunningApps} onCheckedChange={handleDetectByRunningAppsToggle} />
        </div>
      </div>

      {/* Data Storage Locations Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-4">Data Storage Locations</h3>
        <p className="text-sm text-gray-600 mb-6">
          View and access where Meetily stores your data
        </p>

        <div className="space-y-4">
          {/* Database Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Database</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.database || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('database')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Models Location */}
          {/* <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Whisper Models</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.models || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('models')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div> */}

          {/* Recordings Location */}
          <div className="p-4 border rounded-lg bg-gray-50">
            <div className="font-medium mb-2">Meeting Recordings</div>
            <div className="text-sm text-gray-600 mb-3 break-all font-mono text-xs">
              {storageLocations?.recordings || 'Loading...'}
            </div>
            <button
              onClick={() => handleOpenFolder('recordings')}
              className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors"
            >
              <FolderOpen className="w-4 h-4" />
              Open Folder
            </button>
          </div>
        </div>

        <div className="mt-4 p-3 bg-blue-50 rounded-md">
          <p className="text-xs text-blue-800">
            <strong>Note:</strong> Database and models are stored together in your application data directory for unified management.
          </p>
        </div>
      </div>

      {/* Export Section */}
      <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
        <h3 className="text-lg font-semibold text-gray-900 mb-2">Export Meetings</h3>
        <p className="text-sm text-gray-600 mb-4">
          Save every meeting (summary plus timestamped transcript) to a folder you choose. The database stays the source of truth.
          PDF export writes a print-ready page and opens it so you can save it as a PDF from the print dialog.
        </p>

        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => handleExport('markdown', 'Markdown')}
            disabled={exportingFormat !== null}
            className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Download className="w-4 h-4" />
            {exportingFormat === 'markdown' ? 'Exporting…' : 'Markdown'}
          </button>
          <button
            onClick={() => handleExport('docx', 'Word (DOCX)')}
            disabled={exportingFormat !== null}
            className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Download className="w-4 h-4" />
            {exportingFormat === 'docx' ? 'Exporting…' : 'Word (DOCX)'}
          </button>
          <button
            onClick={() => handleExport('pdf', 'PDF (print)')}
            disabled={exportingFormat !== null}
            className="flex items-center gap-2 px-3 py-2 text-sm border border-gray-300 rounded-md hover:bg-gray-100 transition-colors disabled:cursor-not-allowed disabled:opacity-60"
          >
            <Download className="w-4 h-4" />
            {exportingFormat === 'pdf' ? 'Exporting…' : 'PDF (via print)'}
          </button>
        </div>
      </div>

    </div>
  )
}
