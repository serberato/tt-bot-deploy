use super::buffer::{Framer, PrebufferGate};
use super::encoder::new_stream_id;
use super::resampler::IDLE_POLLS_BEFORE_FLUSH;

#[test]
fn framer_yields_full_frames_in_order() {
    let mut framer = Framer::new(16);
    framer.push(&[1, 2, 3, 4, 5]);
    framer.push(&[6, 7, 8]);
    assert_eq!(framer.len(), 8);

    let mut frame = [0i16; 4];
    assert!(framer.pop_frame(&mut frame));
    assert_eq!(frame, [1, 2, 3, 4]);
    assert_eq!(framer.len(), 4);

    assert!(framer.pop_frame(&mut frame));
    assert_eq!(frame, [5, 6, 7, 8]);
    assert_eq!(framer.len(), 0);
}

#[test]
fn framer_pop_fails_when_underfull_and_leaves_data() {
    let mut framer = Framer::new(16);
    framer.push(&[1, 2, 3]);
    let mut frame = [9i16; 4];
    assert!(!framer.pop_frame(&mut frame));
    assert_eq!(frame, [9, 9, 9, 9]);
    assert_eq!(framer.len(), 3);
}

#[test]
fn framer_clear_empties() {
    let mut framer = Framer::new(16);
    framer.push(&[1, 2, 3, 4, 5]);
    framer.clear();
    assert_eq!(framer.len(), 0);
    let mut frame = [0i16; 2];
    assert!(!framer.pop_frame(&mut frame));
}

#[test]
fn stream_ids_are_positive_and_distinct() {
    let a = new_stream_id();
    let b = new_stream_id();
    assert!(a > 0 && b > 0);
    assert_ne!(a, b);
}

#[test]
fn prebuffer_gate_zero_ms_is_always_open() {
    let mut gate = PrebufferGate::new(0);
    assert!(gate.on_data(0));
    assert!(gate.on_idle(0));
    gate.rearm();
    assert!(gate.on_data(0));
}

#[test]
fn prebuffer_gate_holds_until_target_then_latches_open() {
    let mut gate = PrebufferGate::new(400);
    assert!(!gate.on_data(0));
    assert!(!gate.on_data(35279));
    assert!(gate.on_data(35280));
    assert!(gate.on_data(100));
    assert!(gate.on_idle(0));
}

#[test]
fn prebuffer_gate_rearm_closes_again() {
    let mut gate = PrebufferGate::new(100);
    assert!(gate.on_data(8820));
    gate.rearm();
    assert!(!gate.on_data(8819));
    assert!(gate.on_data(8820));
}

#[test]
fn prebuffer_gate_flushes_after_idle_polls_with_data() {
    let mut gate = PrebufferGate::new(400);
    assert!(!gate.on_data(5000));
    for _ in 0..IDLE_POLLS_BEFORE_FLUSH - 1 {
        assert!(!gate.on_idle(5000));
    }
    assert!(gate.on_idle(5000));
}

#[test]
fn prebuffer_gate_stays_armed_while_idle_and_empty() {
    let mut gate = PrebufferGate::new(400);
    for _ in 0..IDLE_POLLS_BEFORE_FLUSH * 3 {
        assert!(!gate.on_idle(0));
    }
    assert!(!gate.on_data(100));
}

#[test]
fn prebuffer_gate_data_resets_idle_streak() {
    let mut gate = PrebufferGate::new(400);
    assert!(!gate.on_data(5000));
    for _ in 0..IDLE_POLLS_BEFORE_FLUSH - 1 {
        assert!(!gate.on_idle(5000));
    }
    assert!(!gate.on_data(6000));
    for _ in 0..IDLE_POLLS_BEFORE_FLUSH - 1 {
        assert!(!gate.on_idle(6000));
    }
    assert!(gate.on_idle(6000));
}
