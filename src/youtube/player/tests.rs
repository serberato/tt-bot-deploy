use super::core::generation_is_stale;
use super::interleave::interleave_to_i16;

#[test]
fn generation_matches_are_fresh_mismatches_are_stale() {
    assert!(!generation_is_stale(5, 5));
    assert!(generation_is_stale(4, 5));
    assert!(generation_is_stale(6, 5));
}

#[test]
fn interleave_pairs_left_and_right() {
    let l = [0.5, -0.5, 0.0];
    let r = [-0.5, 0.5, 1.0];
    let out = interleave_to_i16(&l, &r);
    assert_eq!(out.len(), 6);
    assert_eq!(out[0], (0.5 * 32767.0) as i16);
    assert_eq!(out[1], (-0.5 * 32767.0) as i16);
    assert_eq!(out[2], (-0.5 * 32767.0) as i16);
    assert_eq!(out[3], (0.5 * 32767.0) as i16);
    assert_eq!(out[4], 0);
    assert_eq!(out[5], 32767);
}

#[test]
fn interleave_clamps_overflow() {
    let l = [2.0, -2.0];
    let r = [-2.0, 2.0];
    let out = interleave_to_i16(&l, &r);
    assert_eq!(out, vec![32767, -32767, -32767, 32767]);
}

#[test]
fn interleave_truncates_to_shorter_channel() {
    let l = [0.1, 0.2, 0.3];
    let r = [0.4];
    let out = interleave_to_i16(&l, &r);
    assert_eq!(out.len(), 2);
}

#[test]
fn interleave_empty_returns_empty() {
    let out = interleave_to_i16(&[], &[]);
    assert!(out.is_empty());
}
