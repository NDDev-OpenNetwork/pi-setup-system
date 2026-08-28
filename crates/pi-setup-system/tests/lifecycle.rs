//! The documented round trip, against this crate's own binary, on this system.
//!
//! The three-OS matrix proved the code builds and its unit tests pass on
//! ubuntu, macos and windows. It did not prove that a *target* survives a round
//! trip there -- every lifecycle run against a real directory had happened on
//! Linux. Both Windows defects this project shipped lived in exactly that gap,
//! and neither was visible to a unit test.
//!
//! Everything asked here lives in `harness_runtime::probe`, so the same text
//! runs for all seven systems on all three platforms, and a crate carries only
//! the one fact that is its own: which binary to run.

/// Install, select, back up, hold, restore, restore to a named slot, release,
/// remove -- and a file this provider does not own, present throughout.
#[test]
fn the_documented_round_trip_works_on_this_system() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_pi-setup-system"));
    let problems = harness_runtime::probe::round_trip(exe);
    assert!(
        problems.is_empty(),
        "the round trip disagreed with what this build documents:\n  {}",
        problems.join("\n  ")
    );
}

/// A target this provider was never pointed at is refused, never guessed at.
#[test]
fn a_target_that_is_not_one_is_refused_on_this_system() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_pi-setup-system"));
    let problems = harness_runtime::probe::refuses_a_target_it_should(exe);
    assert!(problems.is_empty(), "{}", problems.join("\n  "));
}

/// Two processes cannot write one target at once, on this system.
///
/// The lock is an in-process claim over `File::try_lock`, and the second is an
/// operating-system primitive -- `flock` here, `LockFileEx` on Windows. A unit
/// test in one process cannot reach it, so this drives real ones.
#[test]
fn one_process_writes_this_target_at_a_time() {
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_pi-setup-system"));
    let problems = harness_runtime::probe::one_writer_at_a_time(exe);
    assert!(problems.is_empty(), "{}", problems.join("\n  "));
}
