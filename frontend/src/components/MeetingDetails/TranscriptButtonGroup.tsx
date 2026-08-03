"use client";

import { useState, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Users } from 'lucide-react';

import { RetranscribeDialog } from './RetranscribeDialog';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);
  const [isDiarizing, setIsDiarizing] = useState(false);

  const handleRetranscribeComplete = useCallback(async () => {
    // Refetch transcripts to show the updated data
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  // Manual speaker diarization (re)run. Progress and completion surface via
  // the global transcript-diarization events (handled in usePaginatedTranscripts),
  // which also refetch the transcript when labels change.
  const handleIdentifySpeakers = useCallback(async () => {
    if (!meetingId || isDiarizing) return;
    setIsDiarizing(true);
    try {
      const ready = await invoke<boolean>('diarization_is_ready');
      if (!ready) {
        toast.warning('Speaker models not downloaded', {
          description: 'Download them from Settings > Transcription > Speaker Identification first.',
        });
        return;
      }
      await invoke('run_speaker_diarization', { meetingId });
    } catch (err) {
      console.error('Manual diarization failed:', err);
      // The transcript-diarization-error event already shows a toast; this
      // fallback covers invoke-level failures (e.g., command rejected).
      toast.error('Speaker identification failed', {
        description: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsDiarizing(false);
    }
  }, [meetingId, isDiarizing]);

  return (
    <div className="flex items-center justify-center w-full gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="sm"
          onClick={onCopyTranscript}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? 'No transcript available' : 'Copy Transcript'}
        >
          <Copy />
          <span className="hidden lg:inline">Copy</span>
        </Button>

        <Button
          size="sm"
          variant="outline"
          className="xl:px-4"
          onClick={() => onOpenMeetingFolder()}
          title="Open Recording Folder"
        >
          <FolderOpen className="xl:mr-2" size={18} />
          <span className="hidden lg:inline">Recording</span>
        </Button>

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 xl:px-4"
            onClick={() => setShowRetranscribeDialog(true)}
            title="Retranscribe to enhance your recorded audio"
          >
            <RefreshCw className="xl:mr-2" size={18} />
            <span className="hidden lg:inline">Enhance</span>
          </Button>
        )}

        {meetingId && meetingFolderPath && transcriptCount > 0 && (
          <Button
            size="sm"
            variant="outline"
            className="xl:px-4"
            onClick={handleIdentifySpeakers}
            disabled={isDiarizing}
            title="Identify who spoke when (on-device speaker diarization)"
          >
            <Users className="xl:mr-2" size={18} />
            <span className="hidden lg:inline">{isDiarizing ? 'Identifying...' : 'Speakers'}</span>
          </Button>
        )}
      </ButtonGroup>

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}
    </div>
  );
}
