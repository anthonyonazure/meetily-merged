import assert from 'node:assert/strict';
import test from 'node:test';

import { checkTranscriptionReadiness, type TauriInvoke } from '../frontend/src/lib/transcription-readiness.ts';

// https://github.com/Zackriya-Solutions/meetily/issues/637
test('Issue #637: a ready Whisper model does not invoke Parakeet checks', async () => {
  const calls: string[] = [];
  const invoke: TauriInvoke = async <T>(command: string) => {
    calls.push(command);
    if (command === 'whisper_get_available_models') {
      return [{ name: 'large-v3', status: 'Available' }] as T;
    }
    return undefined as T;
  };

  const result = await checkTranscriptionReadiness(
    { provider: 'localWhisper', model: 'large-v3' },
    invoke
  );

  assert.deepEqual(result, { ready: true, downloading: false });
  assert.deepEqual(calls, ['whisper_init', 'whisper_get_available_models']);
});

test('reports download state for the configured local model', async () => {
  const invoke: TauriInvoke = async <T>(command: string) => {
    if (command === 'parakeet_get_available_models') {
      return [{ name: 'parakeet-tdt-0.6b-v3-int8', status: { Downloading: { progress: 42 } } }] as T;
    }
    return undefined as T;
  };

  assert.deepEqual(
    await checkTranscriptionReadiness(
      { provider: 'parakeet', model: 'parakeet-tdt-0.6b-v3-int8' },
      invoke
    ),
    { ready: false, downloading: true }
  );
});

test('cloud transcription providers do not require local model runtimes', async () => {
  const invoke: TauriInvoke = async () => {
    throw new Error('local runtime should not be called');
  };

  assert.deepEqual(
    await checkTranscriptionReadiness({ provider: 'deepgram', model: 'nova-3' }, invoke),
    { ready: true, downloading: false }
  );
});
