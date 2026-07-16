export interface TranscriptionModelConfig {
  provider: string;
  model: string;
}

export interface TranscriptionModelInfo {
  name: string;
  status?: 'Available' | 'Missing' | 'Downloading' | { Downloading?: unknown; Error?: string };
}

export interface TranscriptionReadiness {
  ready: boolean;
  downloading: boolean;
}

export type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

function modelStatus(model: TranscriptionModelInfo | undefined): TranscriptionReadiness {
  if (!model) return { ready: false, downloading: false };

  const { status } = model;
  return {
    ready: status === 'Available',
    downloading: status === 'Downloading' || Boolean(status && typeof status === 'object' && 'Downloading' in status),
  };
}

async function localModelReadiness(
  config: TranscriptionModelConfig,
  invoke: TauriInvoke,
  commandPrefix: 'whisper' | 'parakeet'
): Promise<TranscriptionReadiness> {
  await invoke(`${commandPrefix}_init`);
  const models = await invoke<TranscriptionModelInfo[]>(`${commandPrefix}_get_available_models`);
  return modelStatus(models.find(model => model.name === config.model));
}

/**
 * Checks only the runtime required by the configured transcription provider.
 * Cloud providers are validated by their own request path and do not depend on
 * either local model runtime.
 */
export async function checkTranscriptionReadiness(
  config: TranscriptionModelConfig,
  invoke: TauriInvoke
): Promise<TranscriptionReadiness> {
  try {
    if (config.provider === 'localWhisper') {
      return await localModelReadiness(config, invoke, 'whisper');
    }
    if (config.provider === 'parakeet') {
      return await localModelReadiness(config, invoke, 'parakeet');
    }
    return { ready: true, downloading: false };
  } catch (error) {
    console.error(`Failed to check ${config.provider} transcription status:`, error);
    return { ready: false, downloading: false };
  }
}
