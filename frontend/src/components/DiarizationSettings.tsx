'use client';

import { useCallback, useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { Label } from './ui/label';
import { Button } from './ui/button';
import { Switch } from './ui/switch';
import { Progress } from './ui/progress';
import { toast } from 'sonner';
import { CheckCircle2, Download, Users } from 'lucide-react';

interface DiarizationModelInfo {
    ready: boolean;
    segmentation_model_present: boolean;
    embedding_model_present: boolean;
    models_dir: string;
    download_size_mb: number;
}

interface DownloadProgressPayload {
    model: string;
    progress: number;
    downloaded_mb: number;
    total_mb: number;
    status: string;
}

/**
 * Settings card for post-recording speaker diarization: an on/off toggle
 * (default on) and a one-time model download affordance, following the
 * existing Whisper/Parakeet model manager patterns.
 */
export function DiarizationSettings() {
    const [enabled, setEnabled] = useState(true);
    const [modelInfo, setModelInfo] = useState<DiarizationModelInfo | null>(null);
    const [isDownloading, setIsDownloading] = useState(false);
    const [downloadProgress, setDownloadProgress] = useState(0);
    const [downloadLabel, setDownloadLabel] = useState('');

    const refreshModelInfo = useCallback(async () => {
        try {
            const info = await invoke<DiarizationModelInfo>('diarization_get_model_info');
            setModelInfo(info);
        } catch (err) {
            console.error('Failed to fetch diarization model info:', err);
        }
    }, []);

    useEffect(() => {
        const load = async () => {
            try {
                const settings = await invoke<{ enabled: boolean }>('diarization_get_settings');
                setEnabled(settings.enabled);
            } catch (err) {
                console.error('Failed to fetch diarization settings:', err);
            }
            await refreshModelInfo();
        };
        load();
    }, [refreshModelInfo]);

    // Download progress events from the backend
    useEffect(() => {
        let unlisten: (() => void) | undefined;
        let cancelled = false;

        listen<DownloadProgressPayload>('diarization-model-download-progress', (event) => {
            setDownloadProgress(event.payload.progress);
            setDownloadLabel(
                event.payload.model === 'pyannote-segmentation'
                    ? 'Downloading segmentation model...'
                    : 'Downloading speaker embedding model...'
            );
        }).then((fn) => {
            if (cancelled) fn();
            else unlisten = fn;
        });

        return () => {
            cancelled = true;
            if (unlisten) unlisten();
        };
    }, []);

    const handleToggle = async (value: boolean) => {
        const previous = enabled;
        setEnabled(value);
        try {
            await invoke('diarization_set_enabled', { enabled: value });
        } catch (err) {
            console.error('Failed to save diarization setting:', err);
            setEnabled(previous);
            toast.error('Failed to save speaker identification setting');
        }
    };

    const handleDownload = async () => {
        if (isDownloading) return;
        setIsDownloading(true);
        setDownloadProgress(0);
        setDownloadLabel('Starting download...');
        try {
            await invoke('diarization_download_models');
            toast.success('Speaker identification models downloaded');
            await refreshModelInfo();
        } catch (err) {
            console.error('Diarization model download failed:', err);
            toast.error('Model download failed', {
                description: err instanceof Error ? err.message : String(err),
            });
        } finally {
            setIsDownloading(false);
        }
    };

    const modelsReady = modelInfo?.ready ?? false;

    return (
        <div className="border-t border-gray-200 pt-4 mt-4">
            <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                    <Users className="h-4 w-4 text-gray-500" />
                    <div>
                        <Label>Speaker Identification</Label>
                        <p className="text-xs text-muted-foreground">
                            After each recording, label who spoke when ("Speaker 1", "Speaker 2", ...).
                            Runs entirely on this device.
                        </p>
                    </div>
                </div>
                <Switch checked={enabled} onCheckedChange={handleToggle} />
            </div>

            {enabled && (
                <div className="mt-3">
                    {modelsReady ? (
                        <p className="flex items-center gap-1.5 text-xs text-green-600">
                            <CheckCircle2 className="h-3.5 w-3.5" />
                            Speaker models installed
                        </p>
                    ) : isDownloading ? (
                        <div className="space-y-1">
                            <p className="text-xs text-gray-500">{downloadLabel}</p>
                            <Progress value={downloadProgress} className="h-2" />
                        </div>
                    ) : (
                        <div className="flex items-center justify-between gap-2">
                            <p className="text-xs text-muted-foreground">
                                Requires a one-time model download
                                {modelInfo ? ` (~${modelInfo.download_size_mb} MB)` : ''}.
                            </p>
                            <Button
                                type="button"
                                variant="outline"
                                size="sm"
                                onClick={handleDownload}
                            >
                                <Download className="mr-2 h-4 w-4" />
                                Download Models
                            </Button>
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
