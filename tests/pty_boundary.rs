use ptytest::{CommandSpec, ExitStatus, Key, ProtocolProfile, PtyTest, Scenario, Size, TestEnv};
use std::path::PathBuf;
use std::time::Duration;

fn fixture() -> CommandSpec {
    CommandSpec::new(env!("CARGO_BIN_EXE_ptytest-fixture"))
}

fn scenario(label: &str) -> Scenario {
    Scenario::new(label)
        .unwrap()
        .command(fixture())
        .size(Size::new(53, 17).unwrap())
        .environment(TestEnv::hermetic().unwrap())
}

fn deadline(terminal: &PtyTest) -> ptytest::Deadline {
    terminal.deadline(Duration::from_secs(2))
}

#[test]
fn child_has_a_real_controlling_terminal_and_requested_initial_size() {
    let mut terminal = PtyTest::spawn(scenario("kernel ownership")).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "ownership\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "TTY ownership", |screen| {
            screen.contains("OWNERSHIP_OK")
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "size\n").unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "initial kernel size", |screen| {
            screen.contains("SIZE rows=17 columns=53")
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "exit:7\n").unwrap();
    assert_eq!(
        terminal.wait_for_exit(deadline(&terminal)).unwrap(),
        ExitStatus::Code(7)
    );
    assert_eq!(
        terminal.finish(deadline(&terminal)).unwrap(),
        ExitStatus::Code(7)
    );
}

#[test]
fn resize_uses_the_kernel_and_delivers_sigwinch_to_the_foreground_group() {
    let mut terminal = PtyTest::spawn(scenario("kernel resize")).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal.resize(Size::new(50, 10).unwrap()).unwrap();
    terminal.send_text(deadline(&terminal), "winch\n").unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "SIGWINCH delivery", |screen| {
            screen.contains("RESIZE_SIGNAL")
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "size\n").unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "resized kernel dimensions", |screen| {
            screen.contains("SIZE rows=10 columns=50")
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
    assert_eq!(
        terminal.wait_for_exit(deadline(&terminal)).unwrap(),
        ExitStatus::Code(0)
    );
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn mode_aware_cursor_keys_follow_the_independent_terminal_state() {
    let mut terminal = PtyTest::spawn(scenario("application cursor key")).unwrap();
    let baseline = terminal.terminal_baseline();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "application-cursor\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "application cursor mode", |screen| {
            screen.modes().application_cursor
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "key\n").unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "key reader", |screen| {
            screen.contains("KEY_READY")
        })
        .unwrap();
    terminal.send_key(deadline(&terminal), Key::Up).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "application cursor bytes", |screen| {
            screen.contains("KEY:\\x1bOA")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "normal-cursor\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "normal cursor mode", |screen| {
            !screen.modes().application_cursor
        })
        .unwrap();
    terminal.assert_terminal_restored(&baseline).unwrap();
    terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
    terminal.wait_for_exit(deadline(&terminal)).unwrap();
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn every_split_boundary_of_a_csi_input_reaches_the_fixture_unchanged() {
    let sequence = b"\x1b[A";
    for split in 0..=sequence.len() {
        let mut terminal = PtyTest::spawn(scenario(&format!("CSI input split {split}"))).unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
                screen.contains("PTYTEST_READY")
            })
            .unwrap();
        terminal.send_text(deadline(&terminal), "key\n").unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "key reader", |screen| {
                screen.contains("KEY_READY")
            })
            .unwrap();
        terminal
            .send_fragmented(
                deadline(&terminal),
                sequence,
                &[split, sequence.len() - split],
            )
            .unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "CSI input acknowledgement", |screen| {
                screen.contains("KEY:\\x1b[A")
            })
            .unwrap();
        terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
        terminal.wait_for_exit(deadline(&terminal)).unwrap();
        terminal.finish(deadline(&terminal)).unwrap();
    }
}

#[test]
fn approved_queries_receive_a_deterministic_reply() {
    let scenario = scenario("approved query").protocol_profile(ProtocolProfile::xterm_minimal_v1());
    let mut terminal = PtyTest::spawn(scenario).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "query-cpr\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "terminal query reply", |screen| {
            screen.contains("QUERY_REPLY:\\x1b[")
        })
        .unwrap();
    terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
    terminal.wait_for_exit(deadline(&terminal)).unwrap();
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn cpr_uses_the_cursor_at_the_query_and_traces_both_directions() {
    let scenario =
        scenario("CPR cursor timing").protocol_profile(ProtocolProfile::xterm_minimal_v1());
    let mut terminal = PtyTest::spawn(scenario).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "query-cpr-with-trailing-output\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "CPR response", |screen| {
            screen.contains("QUERY_CPR_REPLY:\\x1b[2;2R")
        })
        .unwrap();

    let bundle = match terminal.wait_for_screen(
        terminal.deadline(Duration::from_millis(1)),
        "intentional query trace artifact",
        |_| false,
    ) {
        Err(ptytest::PtyTestError::Timeout {
            artifact_dir: Some(path),
            ..
        }) => path,
        other => panic!("expected timeout artifact, got {other:?}"),
    };
    let events = std::fs::read_to_string(bundle.join("events.jsonl")).unwrap();
    assert!(events.contains("\"kind\":\"terminal-query\""));
    assert!(events.contains("\"kind\":\"terminal-reply\""));
    assert!(events.contains("query-output-offset="));
    assert!(events.contains("input-offset="));
    assert!(
        std::fs::read(bundle.join("input.bin"))
            .unwrap()
            .windows(b"\x1b[2;2R".len())
            .any(|bytes| bytes == b"\x1b[2;2R")
    );
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn direct_exec_failure_is_reported_without_a_child_leak() {
    let scenario = Scenario::new("missing direct executable")
        .unwrap()
        .command(CommandSpec::new("/definitely/not/a/ptytest-fixture"))
        .environment(TestEnv::hermetic().unwrap());
    assert!(matches!(
        PtyTest::spawn(scenario),
        Err(ptytest::PtyTestError::SpawnFailed {
            stage: "execve",
            ..
        })
    ));
}

#[test]
fn tty_generated_sigint_reaches_the_foreground_process_group() {
    let scenario = Scenario::new("tty generated interrupt")
        .unwrap()
        .command(fixture().arg("--wait-sigint"))
        .environment(TestEnv::hermetic().unwrap());
    let mut terminal = PtyTest::spawn(scenario).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "SIGINT readiness", |screen| {
            screen.contains("SIGINT_READY")
        })
        .unwrap();
    terminal
        .send_key(deadline(&terminal), Key::Ctrl('c'))
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "SIGINT handler", |screen| {
            screen.contains("SIGINT_RECEIVED")
        })
        .unwrap();
    assert_eq!(
        terminal.wait_for_exit(deadline(&terminal)).unwrap(),
        ExitStatus::Code(0)
    );
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn terminal_eof_and_hangup_are_distinct_lifecycle_operations() {
    let eof_scenario = Scenario::new("terminal eof")
        .unwrap()
        .command(fixture().arg("--wait-eof"))
        .environment(TestEnv::hermetic().unwrap());
    let mut eof_terminal = PtyTest::spawn(eof_scenario).unwrap();
    eof_terminal
        .wait_for_screen(deadline(&eof_terminal), "EOF readiness", |screen| {
            screen.contains("EOF_READY")
        })
        .unwrap();
    eof_terminal.send_eof(deadline(&eof_terminal)).unwrap();
    eof_terminal
        .wait_for_screen(deadline(&eof_terminal), "EOF delivery", |screen| {
            screen.contains("EOF_RECEIVED")
        })
        .unwrap();
    assert_eq!(
        eof_terminal.wait_for_exit(deadline(&eof_terminal)).unwrap(),
        ExitStatus::Code(0)
    );
    eof_terminal.finish(deadline(&eof_terminal)).unwrap();

    let mut hangup_terminal = PtyTest::spawn(scenario("terminal hangup")).unwrap();
    hangup_terminal
        .wait_for_screen(deadline(&hangup_terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    hangup_terminal.hangup();
    assert_eq!(
        hangup_terminal
            .wait_for_exit(deadline(&hangup_terminal))
            .unwrap(),
        ExitStatus::Signal(libc::SIGHUP)
    );
    hangup_terminal.finish(deadline(&hangup_terminal)).unwrap();
}

#[test]
fn finish_terminates_an_ordinary_descendant_in_the_original_process_group() {
    let mut terminal = PtyTest::spawn(scenario("ordinary descendant cleanup")).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "descendant\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "descendant readiness", |screen| {
            screen.contains("DESCENDANT_READY")
        })
        .unwrap();
    assert_eq!(
        terminal.finish(deadline(&terminal)).unwrap(),
        ExitStatus::Signal(libc::SIGTERM)
    );
}

#[test]
fn finish_terminates_a_descendant_after_the_leader_is_observed_as_a_zombie() {
    let environment = TestEnv::hermetic().unwrap();
    let marker = environment
        .paths()
        .temp()
        .join("ptytest-descendant-terminated");
    let scenario = Scenario::new("zombie leader descendant cleanup")
        .unwrap()
        .command(fixture().arg("--leader-exits-with-descendant"))
        .environment(environment);
    let mut terminal = PtyTest::spawn(scenario).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "descendant readiness", |screen| {
            screen.contains("DESCENDANT_READY")
        })
        .unwrap();
    let descendant = descendant_pid(terminal.raw_output());
    assert_eq!(
        terminal.wait_for_exit(deadline(&terminal)).unwrap(),
        ExitStatus::Code(0)
    );
    terminal.finish(deadline(&terminal)).unwrap();
    let terminated = marker.is_file();
    if !terminated {
        // Keep the regression safe on an implementation that fails before
        // cleanup is fixed: the assertion still fails, but no child survives.
        unsafe { libc::kill(descendant, libc::SIGKILL) };
    }
    assert!(
        terminated,
        "descendant {descendant} was not terminated before the leader was reaped; pgid={} sid={}",
        unsafe { libc::getpgid(descendant) },
        unsafe { libc::getsid(descendant) },
    );
}

#[test]
fn timeout_writes_a_replayable_redacted_failure_bundle() {
    let scenario = Scenario::new("failure bundle")
        .unwrap()
        .command(fixture().secret_env("FIXTURE_TOKEN", "not-a-real-token"))
        .environment(
            TestEnv::hermetic()
                .unwrap()
                .secret_env("APP_SECRET", "also-fake"),
        );
    let mut terminal = PtyTest::spawn(scenario).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    assert!(!terminal
        .wait_for_output(terminal.deadline(Duration::from_millis(50)))
        .unwrap());
    let bundle = match terminal.wait_for_screen(
        terminal.deadline(Duration::from_millis(1)),
        "intentional impossible predicate",
        |_| false,
    ) {
        Err(ptytest::PtyTestError::Timeout {
            artifact_dir: Some(path),
            deadline: Some(timeout),
            elapsed,
            status,
            size,
            ..
        }) => {
            assert_eq!(timeout, Duration::from_millis(1));
            assert!(elapsed >= Duration::from_millis(1));
            assert!(
                elapsed < Duration::from_millis(25),
                "timeout elapsed should describe this wait, got {elapsed:?}"
            );
            assert_eq!(status, ExitStatus::Running);
            assert_eq!(size, Size::new(80, 24).unwrap());
            path
        }
        other => panic!("expected timeout artifact, got {other:?}"),
    };
    assert_bundle(&bundle);
    let command = std::fs::read_to_string(bundle.join("command.txt")).unwrap();
    assert!(command.contains("FIXTURE_TOKEN=<redacted>"));
    assert!(command.contains("APP_SECRET=<redacted>"));
    let events = std::fs::read_to_string(bundle.join("events.jsonl")).unwrap();
    assert!(events.contains("\"version\":1"));
    assert!(events.contains("\"elapsed_ns\":"));
    let configuration = std::fs::read_to_string(bundle.join("configuration.txt")).unwrap();
    assert!(configuration.contains("input-bytes:"));
    assert!(configuration.contains("output-bytes:"));
    assert!(configuration.contains("timeout-deadline-ns: 1000000"));
    let replay = ptytest::replay_failure_bundle(&bundle).unwrap();
    assert!(replay.screen().contains("PTYTEST_READY"));
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn fragmented_utf8_input_and_backpressure_complete_without_a_sleep() {
    for split in 0..=3 {
        let mut terminal =
            PtyTest::spawn(scenario(&format!("fragmented unicode {split}"))).unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
                screen.contains("PTYTEST_READY")
            })
            .unwrap();
        terminal
            .send_text(deadline(&terminal), "unicode\n")
            .unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "unicode reader", |screen| {
                screen.contains("UNICODE_READY")
            })
            .unwrap();
        terminal
            .send_fragmented(deadline(&terminal), "界".as_bytes(), &[split, 3 - split])
            .unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "unicode acknowledgement", |screen| {
                screen.contains("UNICODE:界")
            })
            .unwrap();
        terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
        terminal.wait_for_exit(deadline(&terminal)).unwrap();
        terminal.finish(deadline(&terminal)).unwrap();
    }

    let mut terminal = PtyTest::spawn(scenario("partial write progress")).unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
            screen.contains("PTYTEST_READY")
        })
        .unwrap();
    terminal
        .send_text(deadline(&terminal), "backpressure\n")
        .unwrap();
    terminal
        .wait_for_screen(deadline(&terminal), "backpressure reader", |screen| {
            screen.contains("BACKPRESSURE_READY")
        })
        .unwrap();
    terminal
        .send_bytes(deadline(&terminal), &vec![b'x'; 512 * 1024])
        .unwrap();
    terminal
        .wait_for_screen(
            deadline(&terminal),
            "backpressure acknowledgement",
            |screen| screen.contains("BACKPRESSURE_READ=524288"),
        )
        .unwrap();
    terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
    terminal.wait_for_exit(deadline(&terminal)).unwrap();
    terminal.finish(deadline(&terminal)).unwrap();
}

#[test]
fn seeded_fixture_schedule_preserves_semantic_state_and_cleanup() {
    // The seed is part of the scenario label, so any failure bundle preserves
    // the deterministic operation schedule that exposed it.
    for seed in [0x4d59_5df4_u64, 0x8f24_b1c3_u64] {
        let scenario = scenario(&format!("fixture schedule seed {seed:016x}"))
            .protocol_profile(ProtocolProfile::xterm_minimal_v1());
        let mut terminal = PtyTest::spawn(scenario).unwrap();
        terminal
            .wait_for_screen(deadline(&terminal), "fixture readiness", |screen| {
                screen.contains("PTYTEST_READY")
            })
            .unwrap();
        let mut state = seed;
        for step in 0..12 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            match state % 4 {
                0 => {
                    terminal.send_text(deadline(&terminal), "input\n").unwrap();
                    terminal
                        .wait_for_screen(
                            deadline(&terminal),
                            format!("input schedule step {step}"),
                            |screen| screen.contains("INPUT_ACK"),
                        )
                        .unwrap();
                }
                1 => {
                    let columns = 20 + ((state >> 8) % 60) as u16;
                    let rows = 4 + ((state >> 16) % 20) as u16;
                    let size = Size::new(columns, rows).unwrap();
                    terminal.resize(size).unwrap();
                    terminal
                        .wait_for_screen(
                            deadline(&terminal),
                            format!("resize schedule step {step}"),
                            |screen| screen.size() == size,
                        )
                        .unwrap();
                }
                2 => {
                    terminal
                        .send_text(deadline(&terminal), "alternate-on\n")
                        .unwrap();
                    terminal
                        .wait_for_screen(
                            deadline(&terminal),
                            format!("alternate-screen schedule step {step}"),
                            |screen| screen.modes().alternate_screen && screen.contains("ALT_ON"),
                        )
                        .unwrap();
                    terminal
                        .send_text(deadline(&terminal), "alternate-off\n")
                        .unwrap();
                    terminal
                        .wait_for_screen(
                            deadline(&terminal),
                            format!("primary-screen schedule step {step}"),
                            |screen| !screen.modes().alternate_screen && screen.contains("ALT_OFF"),
                        )
                        .unwrap();
                }
                _ => {
                    terminal
                        .send_text(deadline(&terminal), "split-output\n")
                        .unwrap();
                    terminal
                        .wait_for_screen(
                            deadline(&terminal),
                            format!("fragmented output schedule step {step}"),
                            |screen| screen.contains("split-界"),
                        )
                        .unwrap();
                }
            }
        }
        terminal.send_text(deadline(&terminal), "exit:0\n").unwrap();
        assert_eq!(
            terminal.wait_for_exit(deadline(&terminal)).unwrap(),
            ExitStatus::Code(0)
        );
        terminal.finish(deadline(&terminal)).unwrap();
    }
}

fn assert_bundle(bundle: &PathBuf) {
    for name in [
        "command.txt",
        "configuration.txt",
        "events.jsonl",
        "events.log",
        "input.bin",
        "output.bin",
        "screen.ptytest",
        "terminal-state.txt",
        "exit-status.txt",
        "failure.txt",
    ] {
        assert!(
            bundle.join(name).is_file(),
            "missing {name} in {}",
            bundle.display()
        );
    }
}

fn descendant_pid(output: &[u8]) -> libc::pid_t {
    let output = String::from_utf8_lossy(output);
    let value = output
        .lines()
        .find_map(|line| line.strip_prefix("DESCENDANT_PID="))
        .expect("fixture reported descendant PID");
    value
        .trim()
        .parse()
        .expect("fixture descendant PID is numeric")
}
