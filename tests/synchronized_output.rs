use ptytest::{
    CommandSpec, Deadline, ExitStatus, Key, PtyTest, PtyTestError, Scenario, ScreenSnapshot, TestEnv,
};
use std::cell::Cell;
use std::time::Duration;

const BEGIN: &str = r"\033[?2026h";
const END: &str = r"\033[?2026l";
const CLEAR: &str = r"\033[2J\033[H";

fn shell(label: &str, script: &str) -> PtyTest {
    PtyTest::spawn(
        Scenario::new(label)
            .unwrap()
            .command(CommandSpec::new("/bin/sh").arg("-c").arg(script))
            .environment(TestEnv::hermetic().unwrap()),
    )
    .unwrap()
}

fn soon() -> Deadline {
    Deadline::after(Duration::from_secs(5))
}

fn wait_for_raw_output(terminal: &mut PtyTest, needle: &[u8]) {
    let deadline = soon();
    while !terminal
        .raw_output()
        .windows(needle.len())
        .any(|window| window == needle)
    {
        assert!(
            terminal.wait_for_output(deadline).unwrap(),
            "child never wrote {needle:?}"
        );
    }
}

fn has_old_frame(screen: &ScreenSnapshot) -> bool {
    screen.contains("OLD-A") && screen.contains("OLD-B")
}

fn has_new_frame(screen: &ScreenSnapshot) -> bool {
    screen.contains("NEW-A") && screen.contains("NEW-B")
}

fn failure_bundle(terminal: &mut PtyTest) -> std::path::PathBuf {
    match terminal.wait_for_screen(Deadline::after(Duration::ZERO), "bundle", |_| false) {
        Err(PtyTestError::Timeout {
            artifact_dir: Some(path),
            ..
        }) => path,
        other => panic!("expected a timeout bundle, got {other:?}"),
    }
}

fn stalled_update(label: &str) -> PtyTest {
    let script = format!(
        "stty -echo; printf 'OLD-A\\r\\nOLD-B'; read go; \
         printf '{BEGIN}{CLEAR}NEW-A'; read go; \
         printf '\\r\\nNEW-B{END}'; read go"
    );
    let mut terminal = shell(label, &script);
    terminal
        .wait_for_screen(soon(), "old frame", has_old_frame)
        .unwrap();
    terminal
}

#[test]
fn predicates_never_observe_a_half_written_frame() {
    let script = "printf 'OLD-A\\r\\nOLD-B'; sleep 0.2; \
         printf '\\033[?20'; sleep 0.05; printf '26h\\033[HNEW-A'; sleep 0.3; \
         printf '\\r\\nNEW-B\\033[?2026'; sleep 0.05; printf 'l'; sleep 0.3";
    let mut terminal = shell("half-written frame", script);
    let torn = Cell::new(false);
    let screen = terminal
        .wait_for_screen(soon(), "complete frame", |screen| {
            let blank = !screen.contains("OLD") && !screen.contains("NEW");
            if !blank && !has_old_frame(screen) && !has_new_frame(screen) {
                torn.set(true);
            }
            has_new_frame(screen)
        })
        .unwrap();
    assert!(!torn.get(), "a predicate observed a torn frame");
    assert!(has_new_frame(&screen));
    terminal.finish(soon()).unwrap();
}

#[test]
fn screen_holds_the_previous_frame_until_the_update_ends() {
    let mut terminal = stalled_update("held frame");
    let old = terminal.screen();
    assert!(!terminal.synchronized_update_pending());

    terminal.send_text(soon(), "go\n").unwrap();
    wait_for_raw_output(&mut terminal, b"NEW-A");
    assert!(terminal.synchronized_update_pending());
    assert_eq!(terminal.screen(), old);
    assert!(terminal.live_screen().contains("NEW-A"));
    assert!(!terminal.live_screen().contains("OLD-A"));
    assert!(
        terminal
            .wait_for_quiescence(soon(), Duration::from_millis(50))
            .unwrap()
    );
    assert_eq!(terminal.screen(), old);

    let impossible = terminal.wait_for_screen(
        Deadline::after(Duration::from_millis(100)),
        "partial update",
        |screen| screen.contains("NEW-A"),
    );
    let Err(PtyTestError::Timeout {
        artifact_dir: Some(bundle),
        ..
    }) = impossible
    else {
        panic!("a partial update satisfied a predicate: {impossible:?}");
    };
    let live = std::fs::read_to_string(bundle.join("live-screen.ptytest")).unwrap();
    assert!(live.contains("NEW-A"), "{live}");
    let replayed = ptytest::replay_failure_bundle(&bundle).unwrap();
    assert_eq!(replayed.screen(), &old);

    terminal.send_text(soon(), "go\n").unwrap();
    let changed = terminal
        .wait_for_screen_change(soon(), "update completes")
        .unwrap();
    assert!(has_new_frame(&changed), "{changed}");
    assert!(!terminal.synchronized_update_pending());
    assert_eq!(terminal.screen(), changed);
    terminal.finish(soon()).unwrap();
}

#[test]
fn child_exit_inside_an_update_presents_the_latest_state() {
    let script = format!("printf 'OLD-A\\r\\nOLD-B'; sleep 0.1; printf '{BEGIN}{CLEAR}NEW-A'");
    let mut terminal = shell("exit inside update", &script);
    assert_eq!(terminal.wait_for_exit(soon()).unwrap(), ExitStatus::Code(0));
    assert!(!terminal.synchronized_update_pending());
    assert!(terminal.screen().contains("NEW-A"));
    assert!(!terminal.screen().contains("OLD-A"));
    let bundle = failure_bundle(&mut terminal);
    let events = std::fs::read_to_string(bundle.join("events.log")).unwrap();
    assert!(events.contains("synchronized-update-abandoned pty-closed"), "{events}");
    assert!(!bundle.join("live-screen.ptytest").exists());
    let replayed = ptytest::replay_failure_bundle(&bundle).unwrap();
    assert_eq!(replayed.screen(), &terminal.screen());
    terminal.finish(soon()).unwrap();
}

#[test]
fn hangup_and_finish_inside_an_update_present_the_latest_state() {
    let script = format!("printf 'OLD-A\\r\\nOLD-B{BEGIN}{CLEAR}NEW-A'; sleep 30");
    for (label, hang_up) in [("hangup inside update", true), ("finish inside update", false)] {
        let mut terminal = shell(label, &script);
        wait_for_raw_output(&mut terminal, b"NEW-A");
        assert!(terminal.synchronized_update_pending());
        assert!(!terminal.screen().contains("NEW-A"));
        if hang_up {
            terminal.hangup();
        } else {
            terminal.finish(soon()).unwrap();
        }
        assert!(!terminal.synchronized_update_pending());
        assert!(terminal.screen().contains("NEW-A"));
        assert!(!terminal.screen().contains("OLD-A"));
        if hang_up {
            terminal.finish(soon()).unwrap();
        }
    }
}

#[test]
fn key_encoding_follows_the_childs_latest_modes_during_an_update() {
    let script = format!("stty -echo; printf '{BEGIN}\\033[?1h'; exec cat");
    let mut terminal = shell("keys during update", &script);
    wait_for_raw_output(&mut terminal, b"\x1b[?1h");
    assert!(terminal.synchronized_update_pending());
    assert!(!terminal.terminal_state().modes.application_cursor);
    terminal.send_key(soon(), Key::Up).unwrap();
    assert_eq!(terminal.raw_input(), b"\x1bOA");
    terminal.finish(soon()).unwrap();
}
