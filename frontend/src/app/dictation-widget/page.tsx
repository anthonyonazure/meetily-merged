'use client';

import { useEffect, useState, type CSSProperties } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { Mic, Loader2, CheckCircle2, AlertCircle } from 'lucide-react';

type WidgetState = 'idle' | 'recording' | 'processing' | 'success' | 'error';

interface WidgetPayload {
  state: WidgetState;
  message: string;
  transcript?: string;
  hotkey: string;
}

const DEFAULT_PAYLOAD: WidgetPayload = {
  state: 'idle',
  message: 'Press hotkey to start dictation',
  hotkey: 'fn+space',
};

const DRAG_REGION_STYLE = { WebkitAppRegion: 'drag' } as CSSProperties;

function StateIcon({ state }: { state: WidgetState }) {
  if (state === 'recording') {
    return <Mic className="w-4 h-4 text-rose-500" />;
  }
  if (state === 'processing') {
    return <Loader2 className="w-4 h-4 text-blue-500 animate-spin" />;
  }
  if (state === 'success') {
    return <CheckCircle2 className="w-4 h-4 text-emerald-500" />;
  }
  if (state === 'error') {
    return <AlertCircle className="w-4 h-4 text-amber-500" />;
  }
  return <Mic className="w-4 h-4 text-slate-400" />;
}

export default function DictationWidgetPage() {
  const [payload, setPayload] = useState<WidgetPayload>(DEFAULT_PAYLOAD);

  useEffect(() => {
    let mounted = true;

    invoke<string>('dictation_get_hotkey')
      .then((hotkey) => {
        if (!mounted) return;
        setPayload((prev) => ({ ...prev, hotkey }));
      })
      .catch(() => {
        // Ignore bootstrap read failures.
      });

    const unlistenPromise = listen<WidgetPayload>('dictation-widget-update', (event) => {
      if (!mounted) return;
      setPayload((prev) => ({
        ...prev,
        ...event.payload,
      }));
    });

    return () => {
      mounted = false;
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  return (
    <div
      className="w-full h-full flex items-center justify-center"
      style={{ background: 'transparent' }}
      data-tauri-drag-region
    >
      <div
        className="w-[392px] rounded-2xl border border-white/50 bg-white/90 backdrop-blur-xl shadow-[0_12px_38px_rgba(15,23,42,0.2)] px-4 py-3.5"
        style={DRAG_REGION_STYLE}
      >
        <div className="flex items-start justify-between gap-3">
          <div className="flex items-center gap-2 min-w-0">
            <StateIcon state={payload.state} />
            <div className="min-w-0">
              <p className="text-[13px] font-semibold text-slate-900 truncate">{payload.message}</p>
              <p className="text-[11px] text-slate-500 mt-0.5">
                Hold <span className="font-medium text-slate-700">{payload.hotkey}</span>
              </p>
            </div>
          </div>

          <span className="text-[10px] px-2 py-1 rounded-full bg-slate-100 text-slate-600 uppercase tracking-wide">
            Dictation
          </span>
        </div>

        {payload.transcript && (
          <p className="mt-2 text-[12px] leading-5 text-slate-700 bg-slate-50 rounded-lg px-2.5 py-2 min-h-0 line-clamp-2 break-words">
            {payload.transcript}
          </p>
        )}
      </div>
    </div>
  );
}
