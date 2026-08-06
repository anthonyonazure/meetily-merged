'use client';

/**
 * The meeting-type chip: what kind of meeting this was, and which summary template
 * that choice produced.
 *
 * The chip exists because an automatic template choice is only acceptable if it is
 * visible. It always names the template in force and the reason it was picked, and
 * a correction is one click away.
 */

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Tag } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import type { MeetingTypeValue, MeetingTypeView, TemplateChoiceSource } from '@/types/meetingType';

const CHOICE_COPY: Record<TemplateChoiceSource, string> = {
  client_mapping: "chosen by this client's mapping for this meeting type",
  workspace_mapping: 'chosen by the workspace mapping for this meeting type',
  requested: 'the template you picked; no mapping applies to this type',
  low_confidence:
    'the template you picked. The detected type was not certain enough to override it',
  not_classified: 'the template you picked. This meeting has not been classified yet',
};

interface MeetingTypeChipProps {
  meetingId: string;
  /** The template currently selected in the generator, for context. */
  selectedTemplate?: string;
  refreshKey?: number;
}

export function MeetingTypeChip({
  meetingId,
  selectedTemplate,
  refreshKey = 0,
}: MeetingTypeChipProps) {
  const [view, setView] = useState<MeetingTypeView | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    try {
      setView(
        await invoke<MeetingTypeView>('meeting_type_get', {
          meetingId,
          requestedTemplate: selectedTemplate ?? null,
        }),
      );
    } catch (error) {
      console.error('Failed to load the meeting type:', error);
      setView(null);
    }
  }, [meetingId, selectedTemplate]);

  useEffect(() => {
    void load();
  }, [load, refreshKey]);

  const correct = async (value: MeetingTypeValue) => {
    setBusy(true);
    try {
      setView(
        await invoke<MeetingTypeView>('meeting_type_set', {
          meetingId,
          meetingType: value,
          requestedTemplate: selectedTemplate ?? null,
        }),
      );
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  if (!view) return null;

  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          className="flex items-center gap-1.5 px-2.5 py-1 rounded-md border border-edge bg-wash text-xs text-ink hover:bg-active transition-colors"
          title="Meeting type and the summary template it chose"
        >
          <Tag className="w-3.5 h-3.5 text-muted-ink" />
          <span className={view.label ? '' : 'text-muted-ink'}>
            {view.label ?? 'Type not detected'}
          </span>
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-80 p-3 space-y-3">
        <div>
          <div className="text-sm font-medium text-ink">
            {view.label ?? 'Not classified yet'}
          </div>
          <div className="text-[11px] text-muted-ink">
            {view.source === 'manual'
              ? 'Set by you. The detector will not change it.'
              : view.confidence !== null
                ? `Detected from the transcript, ${Math.round(view.confidence * 100)}% confident${
                    view.is_confident ? '' : ' — not enough to choose a template'
                  }.`
                : 'A type is detected automatically after the first summary is generated.'}
          </div>
        </div>

        <div className="border-t border-edge pt-2">
          <div className="text-[11px] font-medium text-muted-ink mb-1">Template in use</div>
          <div className="text-xs text-ink">{view.template_choice.template_id}</div>
          <div className="text-[11px] text-muted-ink">
            {CHOICE_COPY[view.template_choice.source]}
          </div>
        </div>

        <div className="border-t border-edge pt-2">
          <div className="text-[11px] font-medium text-muted-ink mb-1">Correct the type</div>
          <div className="flex flex-wrap gap-1">
            {view.options.map(option => (
              <button
                key={option.value}
                disabled={busy}
                onClick={() => void correct(option.value)}
                title={option.description}
                className={`px-2 py-0.5 rounded text-[11px] border transition-colors ${
                  view.meeting_type === option.value
                    ? 'border-ink bg-active text-ink'
                    : 'border-edge text-muted-ink hover:bg-wash'
                }`}
              >
                {option.label}
              </button>
            ))}
          </div>
        </div>

        <p className="text-[11px] text-faint border-t border-edge pt-2">
          Regenerating the summary with a template you pick yourself always wins over the
          detected type. Map types to templates in Settings → Summary.
        </p>
      </PopoverContent>
    </Popover>
  );
}
