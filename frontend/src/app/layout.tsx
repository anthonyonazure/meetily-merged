'use client'

import './globals.css'
import { Source_Sans_3 } from 'next/font/google'
import { usePathname } from 'next/navigation'
import Sidebar from '@/components/Sidebar'
import { SidebarProvider } from '@/components/Sidebar/SidebarProvider'
import MainContent from '@/components/MainContent'
import { Toaster, toast } from 'sonner'
import "sonner/dist/styles.css"
import { useState, useEffect, useCallback } from 'react'
import { listen, UnlistenFn } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'
import { TooltipProvider } from '@/components/ui/tooltip'
import { RecordingStateProvider } from '@/contexts/RecordingStateContext'
import { OllamaDownloadProvider } from '@/contexts/OllamaDownloadContext'
import { TranscriptProvider } from '@/contexts/TranscriptContext'
import { ConfigProvider, useConfig } from '@/contexts/ConfigContext'
import { OnboardingProvider } from '@/contexts/OnboardingContext'
import { OnboardingFlow } from '@/components/onboarding'
import { loadBetaFeatures } from '@/types/betaFeatures'
import { DownloadProgressToastProvider } from '@/components/shared/DownloadProgressToast'
import { UpdateCheckProvider } from '@/components/UpdateCheckProvider'
import { RecordingPostProcessingProvider } from '@/contexts/RecordingPostProcessingProvider'
import { ImportAudioDialog, ImportDropOverlay } from '@/components/ImportAudio'
import { ImportDialogProvider } from '@/contexts/ImportDialogContext'
import { AppErrorBoundary } from '@/components/AppErrorBoundary'
import { isAudioExtension, getAudioFormatsDisplayList } from '@/constants/audioFormats'


const sourceSans3 = Source_Sans_3({
  subsets: ['latin'],
  weight: ['400', '500', '600', '700'],
  variable: '--font-source-sans-3',
})

// Module-level component — stable reference across RootLayout re-renders.
// Defined here (not inside RootLayout) so React never sees a new function type
// on re-render, which would cause unmount/remount and break initialization logic.
function ConditionalImportDialog({
  showImportDialog,
  handleImportDialogClose,
  importFilePath,
}: {
  showImportDialog: boolean;
  handleImportDialogClose: (open: boolean) => void;
  importFilePath: string | null;
}) {
  const { betaFeatures } = useConfig();

  // Only mount ImportAudioDialog (and its hooks/listeners) when feature is enabled
  if (!betaFeatures.importAndRetranscribe) {
    return null;
  }

  return (
    <ImportAudioDialog
      open={showImportDialog}
      onOpenChange={handleImportDialogClose}
      preselectedFile={importFilePath}
    />
  );
}

// export { metadata } from './metadata'

export default function RootLayout({
  children,
}: {
  children: React.ReactNode
}) {
  const pathname = usePathname()
  const isDictationWidget = pathname === '/dictation-widget'
  const isOverlayWindow = isDictationWidget

  const [showOnboarding, setShowOnboarding] = useState(false)
  const [onboardingCompleted, setOnboardingCompleted] = useState(false)
  const [onboardingCheckDone, setOnboardingCheckDone] = useState(false)

  // Import audio state
  const [showDropOverlay, setShowDropOverlay] = useState(false)
  const [showImportDialog, setShowImportDialog] = useState(false)
  const [importFilePath, setImportFilePath] = useState<string | null>(null)

  useEffect(() => {
    if (isOverlayWindow) {
      setOnboardingCheckDone(true)
      return
    }

    let cancelled = false

    const checkOnboarding = async () => {
      // Try invoke with a timeout: in dev mode, Tauri IPC may not be ready yet
      const withTimeout = <T,>(promise: Promise<T>, ms: number): Promise<T> =>
        Promise.race([
          promise,
          new Promise<never>((_, reject) => setTimeout(() => reject(new Error('timeout')), ms)),
        ])

      for (let attempt = 0; attempt < 3; attempt++) {
        if (cancelled) return
        try {
          const status = await withTimeout(
            invoke<{ completed: boolean } | null>('get_onboarding_status'),
            3000
          )
          if (cancelled) return
          const isComplete = status?.completed ?? false
          setOnboardingCompleted(isComplete)

          if (!isComplete) {
            console.log('[Layout] Onboarding not completed, showing onboarding flow')
            setShowOnboarding(true)
          } else {
            console.log('[Layout] Onboarding completed, showing main app')
          }
          setOnboardingCheckDone(true)
          return
        } catch (error) {
          console.warn(`[Layout] Onboarding check attempt ${attempt + 1}/3 failed:`, error)
          if (attempt < 2) {
            await new Promise(r => setTimeout(r, 500))
          }
        }
      }

      // All retries failed: show onboarding as fallback
      if (!cancelled) {
        console.error('[Layout] All retries exhausted, showing onboarding as fallback')
        setShowOnboarding(true)
        setOnboardingCompleted(false)
        setOnboardingCheckDone(true)
      }
    }

    checkOnboarding()
    return () => { cancelled = true }
  }, [isOverlayWindow])

  // Sync saved dictation hotkey to backend listener at startup
  useEffect(() => {
    if (isOverlayWindow) return;

    const syncDictationHotkey = async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const savedHotkey = await store.get<string>('dictation_hotkey');

        if (savedHotkey && savedHotkey.trim()) {
          await invoke('dictation_set_hotkey', { hotkey: savedHotkey });
        }
      } catch (error) {
        console.error('[Layout] Failed to sync dictation hotkey:', error);
      }
    };

    syncDictationHotkey();
  }, [isOverlayWindow]);

  // Disable context menu in production
  useEffect(() => {
    if (process.env.NODE_ENV === 'production') {
      const handleContextMenu = (e: MouseEvent) => e.preventDefault();
      document.addEventListener('contextmenu', handleContextMenu);
      return () => document.removeEventListener('contextmenu', handleContextMenu);
    }
  }, []);
  useEffect(() => {
    // Listen for tray recording toggle request
    const unlisten = listen('request-recording-toggle', () => {
      console.log('[Layout] Received request-recording-toggle from tray');

      if (showOnboarding) {
        toast.error("Please complete setup first", {
          description: "You need to finish onboarding before you can start recording."
        });
      } else {
        // If in main app, forward to useRecordingStart via window event
        console.log('[Layout] Forwarding to start-recording-from-sidebar');
        window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'));
      }
    });

    return () => {
      unlisten.then(fn => fn());
    };
  }, [showOnboarding]);

  // Handle file drop for audio import
  const handleFileDrop = useCallback((paths: string[]) => {
    // Check if beta features are enabled (read from localStorage directly since we're outside ConfigProvider)
    const betaFeatures = loadBetaFeatures();

    if (!betaFeatures.importAndRetranscribe) {
      toast.error('Beta feature disabled', {
        description: 'Enable "Import Audio & Retranscribe" in Settings > Beta to use this feature.'
      });
      return;
    }

    // Find the first audio file
    const audioFile = paths.find(p => {
      const ext = p.split('.').pop()?.toLowerCase();
      return !!ext && isAudioExtension(ext);
    });

    if (audioFile) {
      console.log('[Layout] Audio file dropped:', audioFile);
      setImportFilePath(audioFile);
      setShowImportDialog(true);
    } else if (paths.length > 0) {
      toast.error('Please drop an audio file', {
        description: `Supported formats: ${getAudioFormatsDisplayList()}`
      });
    }
  }, []);

  // Listen for drag-drop events
  useEffect(() => {
    if (showOnboarding) return; // Don't handle drops during onboarding

    const unlisteners: UnlistenFn[] = [];
    const cleanedUpRef = { current: false };

    const setupListeners = async () => {
      // Drag enter/over - show overlay only if beta feature is enabled
      const unlistenDragEnter = await listen('tauri://drag-enter', () => {
        if (loadBetaFeatures().importAndRetranscribe) {
          setShowDropOverlay(true);
        }
      });
      if (cleanedUpRef.current) {
        unlistenDragEnter();
        return;
      }
      unlisteners.push(unlistenDragEnter);

      // Drag leave - hide overlay
      const unlistenDragLeave = await listen('tauri://drag-leave', () => {
        setShowDropOverlay(false);
      });
      if (cleanedUpRef.current) {
        unlistenDragLeave();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDragLeave);

      // Drop - process files
      const unlistenDrop = await listen<{ paths: string[] }>('tauri://drag-drop', (event) => {
        setShowDropOverlay(false);
        handleFileDrop(event.payload.paths);
      });
      if (cleanedUpRef.current) {
        unlistenDrop();
        unlisteners.forEach(u => u());
        return;
      }
      unlisteners.push(unlistenDrop);
    };

    setupListeners();

    return () => {
      cleanedUpRef.current = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [showOnboarding, handleFileDrop]);

  // Handle import dialog close
  const handleImportDialogClose = useCallback((open: boolean) => {
    setShowImportDialog(open);
    if (!open) {
      setImportFilePath(null);
    }
  }, []);

  // Handler for ImportDialogProvider - opens import dialog from any child component
  const handleOpenImportDialog = useCallback((filePath?: string | null) => {
    setImportFilePath(filePath ?? null);
    setShowImportDialog(true);
  }, []);

  const handleOnboardingComplete = () => {
    console.log('[Layout] Onboarding completed, reloading app')
    setShowOnboarding(false)
    setOnboardingCompleted(true)
    // Optionally reload the window to ensure all state is fresh
    window.location.reload()
  }

  // Overlay windows (dictation widget): render children directly without
  // providers/sidebar on a transparent background.
  if (isOverlayWindow) {
    return (
      <html lang="en" style={{ background: 'transparent' }}>
        <body className={`${sourceSans3.variable} font-sans antialiased`} style={{ background: 'transparent' }}>
          {children}
          <Toaster position="bottom-right" />
        </body>
      </html>
    )
  }

  return (
    <html lang="en">
      <head>
        {/* Frontend crash reporter, dev only.

            A failure during hydration leaves no trace anywhere we can see: the Rust log
            stays silent, and the error boundary never renders because the tree never
            mounts. The window just sits there — icons drawn by the server HTML but no
            handlers on them, client-only controls missing entirely.

            This runs before any bundle does, so it also catches module-init failures that
            would otherwise prevent a React-level handler from ever installing. It pings
            the dev server, which puts the error in the terminal log. */}
        {process.env.NODE_ENV === 'development' && (
          <script
            dangerouslySetInnerHTML={{
              __html: `(function () {
  var report = function (kind, detail) {
    try {
      fetch('/__frontend_error?kind=' + encodeURIComponent(kind) +
            '&detail=' + encodeURIComponent(String(detail).slice(0, 500)));
    } catch (e) {}
  };
  window.addEventListener('error', function (e) {
    report('error', (e.message || '') + ' @ ' + (e.filename || '') + ':' + (e.lineno || ''));
  });
  window.addEventListener('unhandledrejection', function (e) {
    var r = e.reason;
    report('unhandledrejection', (r && (r.stack || r.message)) || r);
  });
})();`,
            }}
          />
        )}
      </head>
      <body className={`${sourceSans3.variable} font-sans antialiased`}>
        <AppErrorBoundary>
        <RecordingStateProvider>
            <TranscriptProvider>
              <ConfigProvider>
                <OllamaDownloadProvider>
                  <OnboardingProvider>
                    <UpdateCheckProvider>
                      <SidebarProvider>
                        <TooltipProvider>
                          <RecordingPostProcessingProvider>
                            <ImportDialogProvider onOpen={handleOpenImportDialog}>
                              {/* Download progress toast provider - listens for background downloads */}
                              <DownloadProgressToastProvider />

                              {/* Show loading, onboarding, or main app */}
                              {!onboardingCheckDone ? (
                                <div className="fixed inset-0 bg-gray-50 flex items-center justify-center z-50">
                                  <div className="text-center space-y-3">
                                    <div className="w-8 h-8 border-2 border-gray-300 border-t-gray-700 rounded-full animate-spin mx-auto" />
                                    <p className="text-sm text-gray-500">Loading...</p>
                                  </div>
                                </div>
                              ) : showOnboarding ? (
                                <OnboardingFlow onComplete={handleOnboardingComplete} />
                              ) : (
                                <div className="flex">
                                  <Sidebar />
                                  <MainContent>{children}</MainContent>
                                </div>
                              )}
                              {/* Import audio overlay and dialog */}
                              <ImportDropOverlay visible={showDropOverlay} />
                              <ConditionalImportDialog
                                showImportDialog={showImportDialog}
                                handleImportDialogClose={handleImportDialogClose}
                                importFilePath={importFilePath}
                              />
                            </ImportDialogProvider>
                          </RecordingPostProcessingProvider>
                        </TooltipProvider>
                      </SidebarProvider>
                    </UpdateCheckProvider>
                  </OnboardingProvider>

                </OllamaDownloadProvider>
              </ConfigProvider>
            </TranscriptProvider>
          </RecordingStateProvider>
        </AppErrorBoundary>

        {/* Bottom-right: bottom-center sat directly on top of the input box below the
            transcript, so a toast blocked the very control the user reaches for next. */}
        <Toaster position="bottom-right" richColors closeButton />
      </body>
    </html>
  )
}
