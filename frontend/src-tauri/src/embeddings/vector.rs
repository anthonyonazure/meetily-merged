//! Vector arithmetic and the on-disk vector encoding.
//!
//! Pure functions over slices: no database, no Tauri, no I/O, so every rule here
//! is unit testable without a build of the app.
//!
//! Vectors are stored as raw little-endian f32 bytes. Little-endian is fixed
//! explicitly rather than inherited from the host so a database file copied
//! between machines keeps meaning the same thing.

/// Encodes a vector as little-endian f32 bytes for the `embeddings.vector` BLOB.
pub fn encode(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decodes little-endian f32 bytes back into a vector.
///
/// Returns None on a length that is not a multiple of 4, or on a length that
/// disagrees with the stored `dim`, rather than reading garbage: a truncated blob
/// means the row is unusable and should be re-indexed, not silently scored.
pub fn decode(bytes: &[u8], dim: usize) -> Option<Vec<f32>> {
    if bytes.len() != dim * 4 {
        return None;
    }
    let mut values = Vec::with_capacity(dim);
    for chunk in bytes.chunks_exact(4) {
        values.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Some(values)
}

/// Scales a vector to unit length in place. A zero vector is left alone (there is
/// no meaningful direction to normalise it to).
pub fn normalize(vector: &mut [f32]) {
    let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm <= f32::EPSILON {
        return;
    }
    for value in vector.iter_mut() {
        *value /= norm;
    }
}

/// Cosine similarity in [-1, 1]. Returns 0.0 for mismatched lengths or a zero
/// vector, so a bad row scores last instead of poisoning the ranking.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Mean pooling over the token axis, masked by attention so padding tokens do not
/// drag the sentence vector toward zero.
///
/// `token_states` is `[tokens][dim]` flattened row-major, which is the shape the
/// ONNX model's `last_hidden_state` comes back in for a batch of one.
pub fn mean_pool(token_states: &[f32], mask: &[i64], dim: usize) -> Vec<f32> {
    let mut pooled = vec![0.0f32; dim];
    if dim == 0 {
        return pooled;
    }
    let mut counted = 0.0f32;
    for (token, keep) in mask.iter().enumerate() {
        if *keep == 0 {
            continue;
        }
        let start = token * dim;
        let end = start + dim;
        if end > token_states.len() {
            break;
        }
        for (slot, value) in pooled.iter_mut().zip(&token_states[start..end]) {
            *slot += *value;
        }
        counted += 1.0;
    }
    if counted > 0.0 {
        for slot in pooled.iter_mut() {
            *slot /= counted;
        }
    }
    pooled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip_through_the_blob_encoding() {
        let vector = vec![0.5f32, -0.25, 1.0, 0.0];
        let bytes = encode(&vector);
        assert_eq!(bytes.len(), 16);
        assert_eq!(decode(&bytes, 4), Some(vector));
    }

    #[test]
    fn the_encoding_is_little_endian_regardless_of_host() {
        // 1.0f32 is 0x3F800000; little-endian puts the 0x3F last.
        assert_eq!(encode(&[1.0]), vec![0x00, 0x00, 0x80, 0x3F]);
    }

    #[test]
    fn a_truncated_or_mis_dimensioned_blob_decodes_to_nothing() {
        let bytes = encode(&[1.0, 2.0, 3.0]);
        assert_eq!(decode(&bytes, 4), None);
        assert_eq!(decode(&bytes[..7], 3), None);
        assert!(decode(&bytes, 3).is_some());
    }

    #[test]
    fn normalising_gives_unit_length_and_leaves_zero_alone() {
        let mut vector = vec![3.0f32, 4.0];
        normalize(&mut vector);
        assert!((vector[0] - 0.6).abs() < 1e-6);
        assert!((vector[1] - 0.8).abs() < 1e-6);

        let mut zero = vec![0.0f32, 0.0];
        normalize(&mut zero);
        assert_eq!(zero, vec![0.0, 0.0]);
    }

    #[test]
    fn identical_directions_score_one_and_opposites_score_minus_one() {
        assert!((cosine(&[1.0, 0.0], &[2.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosine_refuses_mismatched_or_empty_vectors_instead_of_panicking() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn mean_pooling_ignores_masked_padding_tokens() {
        // Two real tokens ([1,1] and [3,3]) plus one padded token that must not
        // pull the mean toward zero.
        let states = vec![1.0, 1.0, 3.0, 3.0, 100.0, 100.0];
        let pooled = mean_pool(&states, &[1, 1, 0], 2);
        assert_eq!(pooled, vec![2.0, 2.0]);
    }

    #[test]
    fn mean_pooling_an_all_padding_sequence_yields_zeros_not_a_divide_by_zero() {
        let states = vec![1.0, 1.0];
        assert_eq!(mean_pool(&states, &[0], 2), vec![0.0, 0.0]);
    }

    #[test]
    fn mean_pooling_stops_at_the_end_of_the_state_buffer() {
        // A mask longer than the states must not read out of bounds.
        let states = vec![2.0, 2.0];
        assert_eq!(mean_pool(&states, &[1, 1, 1], 2), vec![2.0, 2.0]);
    }
}
