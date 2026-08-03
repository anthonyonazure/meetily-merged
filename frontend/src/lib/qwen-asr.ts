// Types for Qwen3-ASR (GGML-based multilingual ASR) integration
import { invoke } from '@tauri-apps/api/core';

export interface QwenAsrModelInfo {
  name: string;
  path: string;
  size_mb: number;
  speed: string;
  status: QwenAsrModelStatus;
  description: string;
  quantization: QwenAsrQuantizationType;
}

export type QwenAsrQuantizationType = 'Q8_0' | 'F16';

export type QwenAsrModelStatus =
  | 'Available'
  | 'Missing'
  | { Downloading: number }
  | { Error: string }
  | { Corrupted: { file_size: number; expected_min_size: number } };

// User-friendly model display configuration
export interface QwenAsrModelDisplayInfo {
  friendlyName: string;
  icon: string;
  tagline: string;
  recommended?: boolean;
}

export const QWEN_ASR_MODEL_DISPLAY_CONFIG: Record<string, QwenAsrModelDisplayInfo> = {
  'qwen3-asr-1.7b-q8_0': {
    friendlyName: 'Qwen3 ASR 1.7B (Q8)',
    icon: '🧠',
    tagline: 'Multilingual • 1.7B • Recommended quality/speed balance',
    recommended: true,
  },
  'qwen3-asr-1.7b-f16': {
    friendlyName: 'Qwen3 ASR 1.7B (F16)',
    icon: '🎯',
    tagline: 'Multilingual • 1.7B • Highest accuracy',
  },
  'qwen3-asr-0.6b-q8_0': {
    friendlyName: 'Qwen3 ASR 0.6B (Q8)',
    icon: '⚡',
    tagline: 'Multilingual • 0.6B • Faster and lighter',
  },
  'qwen3-asr-0.6b-f16': {
    friendlyName: 'Qwen3 ASR 0.6B (F16)',
    icon: '📦',
    tagline: 'Multilingual • 0.6B • Higher quality than 0.6B Q8',
  },
};

export function getQwenAsrModelDisplayInfo(modelName: string): QwenAsrModelDisplayInfo | null {
  return QWEN_ASR_MODEL_DISPLAY_CONFIG[modelName] || null;
}

export function getQwenAsrModelDisplayName(modelName: string): string {
  const info = QWEN_ASR_MODEL_DISPLAY_CONFIG[modelName];
  return info?.friendlyName || modelName;
}

export function formatFileSize(sizeMb: number): string {
  if (sizeMb >= 1000) {
    return `${(sizeMb / 1000).toFixed(1)}GB`;
  }
  return `${sizeMb}MB`;
}

// Tauri command wrappers for Qwen ASR backend
/// Whether the real Qwen3-ASR engine is compiled into this build.
/// Default builds ship a stub; the Qwen option must stay hidden when false.
export async function isQwenAsrAvailable(): Promise<boolean> {
  try {
    return await invoke<boolean>('qwen_asr_is_available');
  } catch {
    return false;
  }
}

export class QwenAsrAPI {
  static async init(): Promise<void> {
    await invoke('qwen_asr_init');
  }

  static async getAvailableModels(): Promise<QwenAsrModelInfo[]> {
    return await invoke('qwen_asr_get_available_models');
  }

  static async loadModel(modelName: string): Promise<void> {
    await invoke('qwen_asr_load_model', { modelName });
  }

  static async getCurrentModel(): Promise<string | null> {
    return await invoke('qwen_asr_get_current_model');
  }

  static async isModelLoaded(): Promise<boolean> {
    return await invoke('qwen_asr_is_model_loaded');
  }

  static async transcribeAudio(audioData: number[]): Promise<string> {
    return await invoke('qwen_asr_transcribe_audio', { audioData });
  }

  static async getModelsDirectory(): Promise<string> {
    return await invoke('qwen_asr_get_models_directory');
  }

  static async downloadModel(modelName: string): Promise<void> {
    await invoke('qwen_asr_download_model', { modelName });
  }

  static async cancelDownload(modelName: string): Promise<void> {
    await invoke('qwen_asr_cancel_download', { modelName });
  }

  static async deleteModel(modelName: string): Promise<string> {
    return await invoke('qwen_asr_delete_model', { modelName });
  }

  static async hasAvailableModels(): Promise<boolean> {
    return await invoke('qwen_asr_has_available_models');
  }

  static async validateModelReady(): Promise<string> {
    return await invoke('qwen_asr_validate_model_ready');
  }

  static async openModelsFolder(): Promise<void> {
    await invoke('qwen_asr_open_models_folder');
  }
}
