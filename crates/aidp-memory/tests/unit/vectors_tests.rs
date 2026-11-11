use super::*;

const EXACT: f64 = 1e-9;

#[test]
fn blob_round_trips_little_endian() {
    let vector = vec![0.5_f32, -1.25, 3.0, 0.0];
    let bytes = encode_vector(&vector);
    assert_eq!(bytes.len(), 16);
    assert_eq!(&bytes[0..4], &0.5_f32.to_le_bytes());
    assert_eq!(decode_vector(&bytes).unwrap(), vector);
}

#[test]
fn blob_of_odd_length_is_refused() {
    assert!(decode_vector(&[1, 2, 3]).is_err());
}

#[test]
fn cosine_of_identical_vectors_is_one() {
    assert!((cosine(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]) - 1.0).abs() < EXACT);
}

#[test]
fn cosine_of_orthogonal_vectors_is_zero() {
    assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < EXACT);
}

#[test]
fn cosine_of_opposite_vectors_is_minus_one() {
    assert!((cosine(&[1.0, 1.0], &[-1.0, -1.0]) + 1.0).abs() < EXACT);
}

#[test]
fn cosine_of_mismatched_or_null_vectors_is_zero() {
    assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
    assert_eq!(cosine(&[0.0, 0.0], &[1.0, 2.0]), 0.0);
    assert_eq!(cosine(&[], &[]), 0.0);
}
