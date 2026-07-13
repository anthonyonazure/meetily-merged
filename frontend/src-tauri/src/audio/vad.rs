use anyhow::{anyhow, Result};
use silero_rs::{VadConfig, VadSession, VadTransition};
use log::{debug, info, warn};
use std::collections::VecDeque;
use std::time::Duration;

/// Represents a complete speech segment detected by VAD
#[derive(Debug, Clone)]
pub struct SpeechSegment {
    pub samples: Vec<f32>,
    pub start_timestamp_ms: f64,
    pub end_timestamp_ms: f64,
    pub confidence: f32,
}

/// Processes audio in 30ms chunks but returns complete speech segments
pub struct ContinuousVadProcessor {
    session: VadSession,
    chunk_size: usize,
    sample_rate: u32,
    buffer: Vec<f32>,
    speech_segments: VecDeque<SpeechSegment>,
    current_speech: Vec<f32>,
    in_speech: bool,
    processed_samples: usize,
    /// Absolute session time at which the segment being accumulated begins, in ms.
    ///
    /// Silero's own `timestamp_ms` is already absolute session time, so this simply
    /// tracks it — and, after a force-cut, advances past the audio we have emitted. It
    /// replaces a `speech_start_sample` counter that was set to
    /// `processed_samples + timestamp_ms * 16`, i.e. an absolute sample position plus the
    /// same position again: it double-counted, so every force-cut segment was stamped at
    /// roughly twice its true offset.
    current_segment_start_ms: f64,
    // State tracking for smart logging
    last_logged_state: bool,
    /// True once this speech run has been force-cut by the max-duration guard.
    ///
    /// The VAD session accumulates the *whole* run internally and hands it over on
    /// SpeechEnd. If we have already emitted part of that run ourselves, taking the
    /// session's samples would transcribe the same audio twice — so once we have cut,
    /// we use our own remainder instead.
    force_emitted_this_run: bool,
}

/// Longest stretch of unbroken speech that may accumulate before it is transcribed.
///
/// Silero only ends a segment on silence, so a speaker who does not pause produces no
/// output at all — nothing reaches the model, and the screen stays empty until they
/// finally stop. This bounds that wait.
const MAX_SEGMENT_SAMPLES: usize = 16_000 * 5; // 5 seconds at 16kHz

/// How long a silence must last before speech is considered finished, in ms.
///
/// This single number decides transcript granularity. It was hardcoded at 400ms —
/// longer than the pause between sentences in fluent or professional speech, so
/// several sentences merged into one line (real recordings produced segments up to
/// 39 seconds). The default is now 200ms, and the user can tune it in Settings.
static VAD_REDEMPTION_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(200);

pub const VAD_REDEMPTION_MIN_MS: u32 = 100;
pub const VAD_REDEMPTION_MAX_MS: u32 = 400;

/// Set the pause length used by subsequent recordings. Clamped: below ~100ms the VAD
/// fragments mid-word, above ~400ms it merges sentences.
pub fn set_vad_redemption_ms(ms: u32) {
    let clamped = ms.clamp(VAD_REDEMPTION_MIN_MS, VAD_REDEMPTION_MAX_MS);
    VAD_REDEMPTION_MS.store(clamped, std::sync::atomic::Ordering::Relaxed);
    log::info!("VAD redemption time set to {}ms", clamped);
}

pub fn vad_redemption_ms() -> u32 {
    VAD_REDEMPTION_MS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Silero VAD only runs at 16kHz — this is a hard requirement of the model.
const VAD_SAMPLE_RATE: u32 = 16000;

/// Duration of `len` samples at the VAD rate, in milliseconds.
fn samples_to_ms(len: usize) -> f64 {
    (len as f64 / VAD_SAMPLE_RATE as f64) * 1000.0
}

/// Build the Silero config for a given pause length.
///
/// Split out from `ContinuousVadProcessor::new` so the pad invariant below can be
/// asserted across the whole settings range without loading the ONNX model each time.
fn build_vad_config(redemption_time_ms: u32) -> VadConfig {
    let mut config = VadConfig::default();
    config.sample_rate = VAD_SAMPLE_RATE as usize;

    // Silero's own thresholds: lenient enough to ride through natural pauses without
    // fragmenting continuous speech into 40ms shards.
    config.positive_speech_threshold = 0.50;
    config.negative_speech_threshold = 0.35;

    config.redemption_time = Duration::from_millis(redemption_time_ms as u64);

    // CRASH GUARD: neither pad may exceed redemption_time. Silero does not enforce this
    // and *panics* if it is violated, taking the whole audio pipeline down mid-recording
    // — which is exactly what happened once the pause length became user-tunable.
    //
    // On SpeechEnd, silero:
    //   1. slices out `[speech_start - pre_pad .. speech_end + post_pad]`, and
    //   2. drains its buffer up to `speech_end`.
    //
    // But SpeechEnd only fires after `redemption_time` of silence, so:
    //   * only ~redemption_time of audio exists *after* speech_end — a larger post_pad
    //     indexes past the end of the buffer;
    //   * the next utterance can begin ~redemption_time after that drain, so a larger
    //     pre_pad reaches back into audio that has already been dropped.
    // Both paths panic (`get_speech`, silero lib.rs:465-478).
    //
    // The old config (redemption 400 / pre 300 / post 400) held only by luck. Letting the
    // user shorten the pause to 180ms broke it: the pad asked for 400ms of audio that did
    // not exist yet, the pipeline task died, and the transcript stopped dead a few seconds
    // into the meeting while the recording appeared to carry on.
    //
    // Silero's own defaults (redemption 600 / pre 600 / post 0) respect the same rule.
    let pad_ceiling_ms = redemption_time_ms as u64;
    config.pre_speech_pad = Duration::from_millis(pad_ceiling_ms.min(300)); // lead-in context
    config.post_speech_pad = Duration::from_millis(pad_ceiling_ms.min(400)); // trailing context

    // Minimum speech duration. Anything shorter is DISCARDED, so this directly decides
    // whether short interjections survive.
    //
    // 250ms was silently eating one-word replies — 「嗯」「好」「对」, "yes", "OK" — which
    // is a real loss in a meeting transcript. 120ms still clears Whisper's ~100ms floor
    // (the reason the value was raised in the first place) and is well above the 40ms
    // fragments this guard exists to reject. SenseVoice has no such floor at all.
    config.min_speech_time = Duration::from_millis(120);

    config
}

impl ContinuousVadProcessor {
    pub fn new(input_sample_rate: u32, redemption_time_ms: u32) -> Result<Self> {
        let config = build_vad_config(redemption_time_ms);

        debug!(
            "Creating VAD session with: sample_rate={}Hz, redemption={}ms, pre_pad={}ms, post_pad={}ms, min_speech={}ms, input_rate={}Hz",
            VAD_SAMPLE_RATE,
            redemption_time_ms,
            config.pre_speech_pad.as_millis(),
            config.post_speech_pad.as_millis(),
            config.min_speech_time.as_millis(),
            input_sample_rate
        );

        let session = VadSession::new(config)
            .map_err(|e| anyhow!("Failed to create VAD session: {:?}", e))?;

        // VAD uses 30ms chunks at 16kHz (480 samples)
        let vad_chunk_size = (VAD_SAMPLE_RATE as f32 * 0.03) as usize; // 480 samples

        info!("VAD processor created: input={}Hz, vad={}Hz, chunk_size={} samples",
              input_sample_rate, VAD_SAMPLE_RATE, vad_chunk_size);

        Ok(Self {
            session,
            chunk_size: vad_chunk_size,
            sample_rate: input_sample_rate, // Store input rate for resampling ratio in resample_to_16k()
            buffer: Vec::with_capacity(vad_chunk_size * 2),
            speech_segments: VecDeque::new(),
            current_speech: Vec::new(),
            in_speech: false,
            processed_samples: 0,
            current_segment_start_ms: 0.0,
            // Initialize state tracking
            last_logged_state: false,
            force_emitted_this_run: false,
        })
    }

    /// Process incoming audio samples and return any complete speech segments
    /// Handles resampling from input sample rate to 16kHz for VAD processing
    pub fn process_audio(&mut self, samples: &[f32]) -> Result<Vec<SpeechSegment>> {
        // Resample to 16kHz if needed
        let resampled_audio = if self.sample_rate == 16000 {
            samples.to_vec()
        } else {
            self.resample_to_16k(samples)?
        };

        self.buffer.extend_from_slice(&resampled_audio);
        let mut completed_segments = Vec::new();

        // Process complete 30ms chunks (480 samples at 16kHz)
        while self.buffer.len() >= self.chunk_size {
            let chunk: Vec<f32> = self.buffer.drain(..self.chunk_size).collect();
            self.process_chunk(&chunk)?;

            // Extract any completed speech segments
            while let Some(segment) = self.speech_segments.pop_front() {
                completed_segments.push(segment);
            }
        }

        Ok(completed_segments)
    }

    /// Improved resampling from input sample rate to 16kHz with anti-aliasing
    /// Uses linear interpolation and basic low-pass filtering for better quality
    fn resample_to_16k(&self, samples: &[f32]) -> Result<Vec<f32>> {
        if self.sample_rate == 16000 {
            return Ok(samples.to_vec());
        }

        // Calculate downsampling ratio
        let ratio = self.sample_rate as f64 / 16000.0;
        let output_len = (samples.len() as f64 / ratio) as usize;
        let mut resampled = Vec::with_capacity(output_len);

        // Apply simple low-pass filter before downsampling to reduce aliasing
        let cutoff_freq = 0.4; // Normalized frequency (0.4 * Nyquist)
        let mut filtered_samples = Vec::with_capacity(samples.len());
        
        // Simple moving average filter (basic low-pass)
        let filter_size = (self.sample_rate as f64 / (cutoff_freq * self.sample_rate as f64)) as usize;
        let filter_size = std::cmp::max(1, std::cmp::min(filter_size, 5)); // Limit filter size
        
        for i in 0..samples.len() {
            let start = if i >= filter_size { i - filter_size } else { 0 };
            let end = std::cmp::min(i + filter_size + 1, samples.len());
            let sum: f32 = samples[start..end].iter().sum();
            filtered_samples.push(sum / (end - start) as f32);
        }

        // Linear interpolation downsampling
        for i in 0..output_len {
            let source_pos = i as f64 * ratio;
            let source_index = source_pos as usize;
            let fraction = source_pos - source_index as f64;
            
            if source_index + 1 < filtered_samples.len() {
                // Linear interpolation
                let sample1 = filtered_samples[source_index];
                let sample2 = filtered_samples[source_index + 1];
                let interpolated = sample1 + (sample2 - sample1) * fraction as f32;
                resampled.push(interpolated);
            } else if source_index < filtered_samples.len() {
                resampled.push(filtered_samples[source_index]);
            }
        }

        debug!("Resampled from {} samples ({}Hz) to {} samples (16kHz) with anti-aliasing",
               samples.len(), self.sample_rate, resampled.len());

        Ok(resampled)
    }

    /// Flush any remaining audio and return final speech segments
    pub fn flush(&mut self) -> Result<Vec<SpeechSegment>> {
        debug!("VAD flush: in_speech={}, current_speech_len={}, buffer_len={}, speech_segments_queued={}",
              self.in_speech, self.current_speech.len(), self.buffer.len(), self.speech_segments.len());

        let mut completed_segments = Vec::new();

        // Process any remaining buffered audio
        if !self.buffer.is_empty() {
            let remaining = self.buffer.clone();
            self.buffer.clear();

            // Pad to chunk size if needed
            let mut padded_chunk = remaining;
            if padded_chunk.len() < self.chunk_size {
                padded_chunk.resize(self.chunk_size, 0.0);
            }

            self.process_chunk(&padded_chunk)?;
        }

        // Force end any ongoing speech
        if self.in_speech && !self.current_speech.is_empty() {
            // Derive the span from the audio we actually hold, so it stays consistent with
            // the samples even when this run was already force-cut.
            let start_ms = self.current_segment_start_ms;
            let end_ms = start_ms + samples_to_ms(self.current_speech.len());

            debug!("VAD flush: Force-ending speech - start={}ms, end={}ms, duration={}ms, samples={}",
                  start_ms, end_ms, end_ms - start_ms, self.current_speech.len());

            let segment = SpeechSegment {
                samples: self.current_speech.clone(),
                start_timestamp_ms: start_ms,
                end_timestamp_ms: end_ms,
                confidence: 0.8, // Estimated confidence for forced end
            };

            self.speech_segments.push_back(segment);
            self.current_speech.clear();
            self.in_speech = false;
        }

        // Extract all remaining segments
        while let Some(segment) = self.speech_segments.pop_front() {
            completed_segments.push(segment);
        }

        Ok(completed_segments)
    }

    fn process_chunk(&mut self, chunk: &[f32]) -> Result<()> {
        // Track accumulated speech buffer size to detect memory issues
        let current_speech_size = self.current_speech.len();
        if current_speech_size > 1_000_000 {
            // More than ~62 seconds of accumulated speech at 16kHz
            warn!("VAD: Accumulated speech buffer is large: {} samples ({:.1}s) - possible memory issue",
                  current_speech_size, current_speech_size as f64 / 16000.0);
        }

        let transitions = self.session.process(chunk)
            .map_err(|e| anyhow!("VAD processing failed: {}", e))?;

        // Log transitions for debugging
        if !transitions.is_empty() {
            debug!("VAD transitions at sample {}: {} transitions", self.processed_samples, transitions.len());
        }

        // Handle VAD transitions
        for transition in transitions {
            match transition {
                VadTransition::SpeechStart { timestamp_ms } => {
                    // Only log if state changed
                    if !self.last_logged_state {
                        debug!("VAD: Speech started at {}ms", timestamp_ms);
                        self.last_logged_state = true;
                    }
                    self.in_speech = true;
                    // Silero reports absolute session time, so take it as-is.
                    self.current_segment_start_ms = timestamp_ms as f64;
                    self.current_speech.clear();
                    self.force_emitted_this_run = false;
                }
                VadTransition::SpeechEnd { start_timestamp_ms, end_timestamp_ms, samples } => {
                    // Only log if we were previously in speech state
                    if self.last_logged_state {
                        debug!("VAD: Speech ended at {}ms (duration: {}ms)", end_timestamp_ms, end_timestamp_ms - start_timestamp_ms);
                        self.last_logged_state = false;
                    }
                    self.in_speech = false;

                    // Use samples from the VAD transition if available, otherwise our own
                    // accumulation. But once this run has been force-cut, the session's
                    // samples still contain the part we already emitted — taking them
                    // would transcribe that audio a second time.
                    //
                    // The timestamps must follow the samples. Silero reports the span of
                    // the *whole* run, so after a force-cut its start is far earlier than
                    // the remainder we are about to emit: a 2.1s tail was being stamped as
                    // a 12.5s segment starting ten seconds too early, which lands the
                    // transcript line at the wrong time and out of order.
                    let (speech_samples, start_ms, end_ms) = if self.force_emitted_this_run {
                        let samples = std::mem::take(&mut self.current_speech);
                        let start = self.current_segment_start_ms;
                        let end = start + samples_to_ms(samples.len());
                        (samples, start, end)
                    } else if !samples.is_empty() {
                        (samples, start_timestamp_ms as f64, end_timestamp_ms as f64)
                    } else {
                        let samples = std::mem::take(&mut self.current_speech);
                        (samples, start_timestamp_ms as f64, end_timestamp_ms as f64)
                    };

                    if !speech_samples.is_empty() {
                        let segment = SpeechSegment {
                            samples: speech_samples,
                            start_timestamp_ms: start_ms,
                            end_timestamp_ms: end_ms,
                            confidence: 0.9, // VAD confidence
                        };

                        info!("VAD: Completed speech segment: {:.1}ms duration, {} samples",
                              end_ms - start_ms, segment.samples.len());

                        self.speech_segments.push_back(segment);
                    }

                    self.current_speech.clear();
                }
            }
        }

        // Accumulate speech if we're currently in a speech state
        if self.in_speech {
            self.current_speech.extend_from_slice(chunk);

            // Force a cut if the speaker has not paused for a long time, so the
            // transcript keeps appearing instead of waiting for them to stop.
            if self.current_speech.len() >= MAX_SEGMENT_SAMPLES {
                let emitted_ms = samples_to_ms(self.current_speech.len());
                let start_ms = self.current_segment_start_ms;
                let end_ms = start_ms + emitted_ms;

                info!(
                    "VAD: speech ran past {:.1}s without a pause — cutting so the transcript keeps up",
                    MAX_SEGMENT_SAMPLES as f64 / 16000.0
                );

                self.speech_segments.push_back(SpeechSegment {
                    samples: std::mem::take(&mut self.current_speech),
                    start_timestamp_ms: start_ms,
                    end_timestamp_ms: end_ms,
                    confidence: 0.85,
                });

                // The next chunk of this run starts where this one ended.
                self.current_segment_start_ms = end_ms;
                self.force_emitted_this_run = true;
            }
        }

        self.processed_samples += chunk.len();
        Ok(())
    }
}

/// Legacy function for backward compatibility - now uses the optimized approach
pub fn extract_speech_16k(samples_mono_16k: &[f32]) -> Result<Vec<f32>> {
    let mut processor = ContinuousVadProcessor::new(16000, 400)?;

    // Process all audio
    let mut all_segments = processor.process_audio(samples_mono_16k)?;
    let final_segments = processor.flush()?;
    all_segments.extend(final_segments);

    // Concatenate all speech segments
    let mut result = Vec::new();
    let num_segments = all_segments.len();
    for segment in &all_segments {
        result.extend_from_slice(&segment.samples);
    }

    // Apply balanced energy filtering for very short segments
    if result.len() < 1600 { // Less than 100ms at 16kHz
        let input_energy: f32 = samples_mono_16k.iter().map(|&x| x * x).sum::<f32>() / samples_mono_16k.len() as f32;
        let rms = input_energy.sqrt();
        let peak = samples_mono_16k.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);

        // Fallback energy gate, used only when VAD extracted almost nothing.
        //
        // NB: the comment here used to claim thresholds of 0.03/0.08 "to catch quiet
        // speech" while the code actually gated at 0.2/0.20 — roughly 7x higher, and
        // well above normal speech (RMS ~0.02-0.1). The values below are the real ones.
        // This is the legacy `extract_speech_16k` path; live recording goes through
        // ContinuousVadProcessor and never reaches here.
        if rms < 0.03 || peak < 0.08 {
            info!("-----VAD detected silence/noise (RMS: {:.6}, Peak: {:.6}), skipping to prevent hallucinations-----", rms, peak);
            return Ok(Vec::new());
        } else {
            info!("VAD detected speech with sufficient energy (RMS: {:.6}, Peak: {:.6})", rms, peak);
            return Ok(samples_mono_16k.to_vec());
        }
    }

    debug!("VAD: Processed {} samples, extracted {} speech samples from {} segments",
           samples_mono_16k.len(), result.len(), num_segments);

    Ok(result)
}

/// Simple convenience function to get speech chunks from audio
/// Uses the optimized ContinuousVadProcessor with configurable redemption time
pub fn get_speech_chunks(samples_mono_16k: &[f32], redemption_time_ms: u32) -> Result<Vec<SpeechSegment>> {
    get_speech_chunks_with_progress(samples_mono_16k, redemption_time_ms, |_, _| true)
}

/// Get speech chunks with progress callback and cancellation support
/// The callback receives (progress_percent, segments_found) and returns false to cancel
pub fn get_speech_chunks_with_progress<F>(
    samples_mono_16k: &[f32],
    redemption_time_ms: u32,
    mut progress_callback: F,
) -> Result<Vec<SpeechSegment>>
where
    F: FnMut(u32, usize) -> bool,
{
    let mut processor = ContinuousVadProcessor::new(16000, redemption_time_ms)?;

    let total_samples = samples_mono_16k.len();

    // For large files (>1 minute at 16kHz = 960,000 samples), process in chunks with progress logging
    const LARGE_FILE_THRESHOLD: usize = 960_000;
    const CHUNK_SIZE: usize = 160_000; // 10 seconds at 16kHz

    let mut all_segments = Vec::new();

    if total_samples > LARGE_FILE_THRESHOLD {
        info!("VAD: Processing large file ({} samples = {:.1}s), will log progress...",
              total_samples, total_samples as f64 / 16000.0);

        let mut processed = 0;
        let mut last_progress = 0u32;
        let mut chunk_count = 0;
        let total_chunks = (total_samples + CHUNK_SIZE - 1) / CHUNK_SIZE;

        for chunk in samples_mono_16k.chunks(CHUNK_SIZE) {
            chunk_count += 1;

            let start_time = std::time::Instant::now();
            let segments = processor.process_audio(chunk)?;
            let elapsed = start_time.elapsed();

            // Debug log for chunk processing details
            debug!("VAD: Chunk {}/{} processed in {:?}, found {} segments",
                  chunk_count, total_chunks, elapsed, segments.len());

            // Warn if chunk processing took too long (>1 second)
            if elapsed.as_secs() > 1 {
                warn!("VAD: Chunk {} took {:?} - possible performance issue", chunk_count, elapsed);
            }

            all_segments.extend(segments);

            processed += chunk.len();
            let progress = ((processed * 100) / total_samples) as u32;

            // Call progress callback every 5%
            if progress >= last_progress + 5 {
                debug!("VAD: Progress {}% ({} segments found so far)", progress, all_segments.len());

                // Check for cancellation
                if !progress_callback(progress, all_segments.len()) {
                    info!("VAD: Cancelled by callback at {}%", progress);
                    return Err(anyhow!("VAD processing cancelled"));
                }

                last_progress = progress;
            }
        }

        let final_segments = processor.flush()?;
        all_segments.extend(final_segments);

        info!("VAD: Complete! Found {} speech segments", all_segments.len());
    } else {
        // Small file - process all at once
        all_segments = processor.process_audio(samples_mono_16k)?;
        let final_segments = processor.flush()?;
        all_segments.extend(final_segments);
    }

    Ok(all_segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate synthetic speech-like audio with alternating speech/silence
    fn generate_test_audio_with_speech(duration_seconds: f32, sample_rate: u32) -> Vec<f32> {
        let total_samples = (duration_seconds * sample_rate as f32) as usize;
        let mut samples = vec![0.0f32; total_samples];

        // Create speech-like patterns: bursts of sine waves with varying amplitude
        // Speech every 10 seconds for 5 seconds
        let speech_interval = 10.0; // seconds between speech starts
        let speech_duration = 5.0;  // seconds of speech

        for i in 0..total_samples {
            let time = i as f32 / sample_rate as f32;
            let cycle_time = time % speech_interval;

            // Speech occurs in the first `speech_duration` seconds of each cycle
            if cycle_time < speech_duration {
                // Generate speech-like signal: multiple frequencies with amplitude modulation
                let freq1 = 200.0 + (time * 50.0).sin() * 100.0; // Varying fundamental
                let freq2 = freq1 * 2.0; // Harmonic
                let freq3 = freq1 * 3.0; // Another harmonic

                let amplitude = 0.3 + 0.1 * (time * 5.0).sin(); // Amplitude modulation
                samples[i] = amplitude * (
                    0.5 * (2.0 * std::f32::consts::PI * freq1 * time).sin() +
                    0.3 * (2.0 * std::f32::consts::PI * freq2 * time).sin() +
                    0.2 * (2.0 * std::f32::consts::PI * freq3 * time).sin()
                );
            }
            // else: silence (already 0.0)
        }

        samples
    }

    #[test]
    fn test_vad_chunked_vs_single_processing() {
        // Generate 60 seconds of audio with speech patterns at 16kHz
        let audio = generate_test_audio_with_speech(60.0, 16000);
        println!("Generated {} samples ({:.1}s)", audio.len(), audio.len() as f32 / 16000.0);

        // Process all at once (like small files)
        let segments_single = get_speech_chunks(&audio, 2000).expect("Single processing failed");
        println!("Single processing found {} segments", segments_single.len());

        // Process in chunks (like large files)
        let segments_chunked = get_speech_chunks_with_progress(&audio, 2000, |progress, segments| {
            println!("Chunked progress: {}%, {} segments", progress, segments);
            true // Don't cancel
        }).expect("Chunked processing failed");
        println!("Chunked processing found {} segments", segments_chunked.len());

        // Both should find the same number of segments (approximately)
        // Allow some variance due to chunk boundary effects
        let diff = (segments_single.len() as i32 - segments_chunked.len() as i32).abs();
        assert!(diff <= 1,
            "Chunked and single processing found different segment counts: {} vs {} (diff: {})",
            segments_single.len(), segments_chunked.len(), diff);
    }

    #[test]
    fn test_vad_large_file_progress() {
        // Generate 120 seconds (2 minutes) of audio - triggers large file threshold
        let audio = generate_test_audio_with_speech(120.0, 16000);
        let total_samples = audio.len();
        println!("Generated {} samples ({:.1}s)", total_samples, total_samples as f32 / 16000.0);

        // This should trigger the large file path (>960,000 samples)
        assert!(total_samples > 960_000, "Audio should be large enough to trigger chunked processing");

        let mut progress_updates = Vec::new();
        let segments = get_speech_chunks_with_progress(&audio, 2000, |progress, segments| {
            progress_updates.push((progress, segments));
            true // Don't cancel
        }).expect("Processing failed");

        println!("Found {} segments with {} progress updates", segments.len(), progress_updates.len());

        // The synthetic signal is not real speech, so Silero may merge it into
        // one long segment. This test is specifically for the large-file path:
        // it must still emit speech and report monotonic progress through 100%.
        assert!(!segments.is_empty(), "Expected at least one speech segment");
        assert!(
            segments.iter().all(|segment| !segment.samples.is_empty()
                && segment.end_timestamp_ms > segment.start_timestamp_ms),
            "Expected all speech segments to contain audio with positive duration"
        );

        // Should have received progress updates
        assert!(!progress_updates.is_empty(), "Expected progress updates for large file");
        assert_eq!(
            progress_updates.last().map(|(progress, _)| *progress),
            Some(100),
            "Expected progress to reach 100%"
        );
        assert!(
            progress_updates
                .windows(2)
                .all(|pair| pair[0].0 < pair[1].0),
            "Expected progress updates to increase monotonically: {:?}",
            progress_updates
        );
    }

    #[test]
    fn test_vad_cancellation() {
        let audio = generate_test_audio_with_speech(120.0, 16000);

        // Cancel at 50%
        let result = get_speech_chunks_with_progress(&audio, 2000, |progress, _| {
            progress < 50 // Cancel when reaching 50%
        });

        // Should return error due to cancellation
        assert!(result.is_err(), "Expected cancellation error");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("cancelled"), "Error should mention cancellation: {}", err_msg);
    }

    #[test]
    fn test_vad_continuous_processor_state_across_chunks() {
        // Test that VAD state is correctly maintained across chunk boundaries
        let mut processor = ContinuousVadProcessor::new(16000, 2000).expect("Failed to create processor");

        // Generate audio with a speech segment that spans a chunk boundary
        let chunk_size = 160_000; // 10 seconds
        let audio = generate_test_audio_with_speech(30.0, 16000); // 30 seconds

        // Process in 10-second chunks
        let mut all_segments = Vec::new();
        for (i, chunk) in audio.chunks(chunk_size).enumerate() {
            let segments = processor.process_audio(chunk).expect("Processing failed");
            println!("Chunk {}: processed {} samples, found {} segments", i, chunk.len(), segments.len());
            all_segments.extend(segments);
        }

        // Flush remaining
        let final_segments = processor.flush().expect("Flush failed");
        all_segments.extend(final_segments);

        println!("Total segments found: {}", all_segments.len());

        // Should find speech segments
        assert!(all_segments.len() >= 1, "Expected at least 1 speech segment");
    }

    /// speech / short gap / speech — the pattern that crashed the recording.
    fn generate_speech_gap_speech(speech_ms: u32, gap_ms: u32, sample_rate: u32) -> Vec<f32> {
        let tone = |samples: usize, offset: usize| -> Vec<f32> {
            (0..samples)
                .map(|i| {
                    let time = (i + offset) as f32 / sample_rate as f32;
                    let f1 = 200.0 + (time * 50.0).sin() * 100.0;
                    0.3 * (0.5 * (2.0 * std::f32::consts::PI * f1 * time).sin()
                        + 0.3 * (2.0 * std::f32::consts::PI * f1 * 2.0 * time).sin()
                        + 0.2 * (2.0 * std::f32::consts::PI * f1 * 3.0 * time).sin())
                })
                .collect()
        };

        let speech_samples = (speech_ms as usize * sample_rate as usize) / 1000;
        let gap_samples = (gap_ms as usize * sample_rate as usize) / 1000;

        let mut audio = tone(speech_samples, 0);
        audio.extend(std::iter::repeat(0.0).take(gap_samples));
        audio.extend(tone(speech_samples, speech_samples + gap_samples));
        // Trailing silence so the final utterance is closed out rather than left open.
        audio.extend(std::iter::repeat(0.0).take(sample_rate as usize));
        audio
    }

    /// Every pause length the user can pick must survive a real speech/pause/speech run.
    ///
    /// Silero panics — it does not error — when a pad reaches outside its audio buffer,
    /// and that panic killed the pipeline task mid-meeting: the transcript stopped after
    /// a few seconds while the recording appeared to carry on. Both pads are therefore
    /// clamped to `redemption_time`, and this test walks the whole settings range plus a
    /// gap short enough to exercise the buffer-drain edge. Before the clamp, 100ms and
    /// 200ms panicked here.
    #[test]
    fn vad_does_not_panic_at_any_user_selectable_pause_length() {
        let audio = generate_speech_gap_speech(1000, 250, 16000);

        for redemption_ms in [
            VAD_REDEMPTION_MIN_MS,
            200, // the shipped default
            300,
            VAD_REDEMPTION_MAX_MS,
        ] {
            let segments = get_speech_chunks(&audio, redemption_ms)
                .unwrap_or_else(|e| panic!("VAD failed at redemption={}ms: {}", redemption_ms, e));

            assert!(
                !segments.is_empty(),
                "redemption={}ms found no speech at all",
                redemption_ms
            );
            assert!(
                segments.iter().all(|s| !s.samples.is_empty()),
                "redemption={}ms emitted an empty segment",
                redemption_ms
            );
        }
    }

    /// A force-cut segment's timestamps must describe the audio it actually carries.
    ///
    /// Continuous speech is cut every 5s so the transcript keeps flowing, and the pieces
    /// used to be stamped from a `speech_start_sample` counter that added an absolute
    /// sample position to the same position expressed in ms — it double-counted, so each
    /// cut landed at roughly twice its true offset. These timestamps become the transcript
    /// line's `audio_start_time`, so the lines were placed at the wrong time in the meeting.
    ///
    /// Driven white-box: Silero does not reliably classify synthetic tones as speech (the
    /// 60s tone in the tests above yields a single 1.2s segment), so it cannot exercise a
    /// 5s force-cut. The accumulator runs off our own `in_speech` flag, so setting it and
    /// feeding audio reaches exactly the bookkeeping under test.
    #[test]
    fn force_cut_segments_are_stamped_to_match_their_audio() {
        let mut processor = ContinuousVadProcessor::new(16000, 200).expect("VAD processor");

        const SPEECH_STARTED_AT_MS: f64 = 1000.0;
        processor.in_speech = true;
        processor.current_segment_start_ms = SPEECH_STARTED_AT_MS;

        // 12s of unbroken speech → two 5s force-cuts, with the remainder still buffered.
        let segments = processor
            .process_audio(&vec![0.05f32; 16_000 * 12])
            .expect("VAD failed");

        assert_eq!(
            segments.len(),
            2,
            "12s of unbroken speech should be cut into two 5s segments, got {}",
            segments.len()
        );

        let mut expected_start_ms = SPEECH_STARTED_AT_MS;
        for (i, seg) in segments.iter().enumerate() {
            let reported_ms = seg.end_timestamp_ms - seg.start_timestamp_ms;
            let actual_ms = samples_to_ms(seg.samples.len());

            assert!(
                (reported_ms - actual_ms).abs() < 1.0,
                "segment {} claims {:.0}ms but carries {:.0}ms of audio ({} samples)",
                i,
                reported_ms,
                actual_ms,
                seg.samples.len()
            );
            assert!(
                (seg.start_timestamp_ms - expected_start_ms).abs() < 1.0,
                "segment {} starts at {:.0}ms, expected {:.0}ms — cuts must be contiguous",
                i,
                seg.start_timestamp_ms,
                expected_start_ms
            );
            expected_start_ms = seg.end_timestamp_ms;
        }

        // The tail carries on from where the last cut ended, rather than rewinding to the
        // start of the run — this is the path that stamped a 2.1s tail as a 12.5s segment.
        let tail = processor.flush().expect("flush failed");
        assert_eq!(tail.len(), 1, "expected the buffered remainder to flush");
        assert!(
            (tail[0].start_timestamp_ms - expected_start_ms).abs() < 1.0,
            "tail starts at {:.0}ms, expected {:.0}ms",
            tail[0].start_timestamp_ms,
            expected_start_ms
        );
        assert!(
            (tail[0].end_timestamp_ms - tail[0].start_timestamp_ms
                - samples_to_ms(tail[0].samples.len()))
            .abs()
                < 1.0,
            "tail's span does not match the audio it carries"
        );
    }

    /// The invariant behind the clamp above, asserted directly on the config.
    #[test]
    fn vad_pads_never_exceed_the_redemption_time() {
        for redemption_ms in VAD_REDEMPTION_MIN_MS..=VAD_REDEMPTION_MAX_MS {
            let config = build_vad_config(redemption_ms);

            assert!(
                config.pre_speech_pad <= config.redemption_time,
                "pre_speech_pad {:?} exceeds redemption {:?} — silero would reach into drained audio",
                config.pre_speech_pad,
                config.redemption_time
            );
            assert!(
                config.post_speech_pad <= config.redemption_time,
                "post_speech_pad {:?} exceeds redemption {:?} — silero would index past its buffer",
                config.post_speech_pad,
                config.redemption_time
            );
        }
    }

    #[test]
    fn test_vad_400ms_vs_2000ms_segmentation() {
        // Demonstrates why 2000ms redemption is needed for batch processing:
        // 400ms creates excessive fragmentation, 2000ms bridges natural pauses.
        //
        // Audio pattern: 60s with 5s speech / 5s silence cycles
        // Natural pauses within speech (sentence gaps) are 500ms-1.5s
        let audio = generate_test_audio_with_speech(60.0, 16000);

        let segments_400 = get_speech_chunks(&audio, 400).expect("400ms processing failed");
        let segments_2000 = get_speech_chunks(&audio, 2000).expect("2000ms processing failed");

        println!(
            "400ms redemption: {} segments, 2000ms redemption: {} segments",
            segments_400.len(),
            segments_2000.len()
        );

        // 2000ms should produce fewer or equal segments (bridges more pauses)
        assert!(
            segments_2000.len() <= segments_400.len(),
            "2000ms redemption ({} segments) should not produce more segments than 400ms ({} segments)",
            segments_2000.len(),
            segments_400.len()
        );

        // Verify segments have reasonable durations with 2000ms
        for (i, seg) in segments_2000.iter().enumerate() {
            let duration_ms = seg.end_timestamp_ms - seg.start_timestamp_ms;
            println!("2000ms segment {}: {:.0}ms duration", i, duration_ms);
            // Each segment should be at least 250ms (min_speech_time)
            assert!(duration_ms >= 200.0, "Segment {} too short: {:.0}ms", i, duration_ms);
        }
    }
}

