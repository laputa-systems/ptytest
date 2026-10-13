use ptytest::{CommandSpec, Deadline, ExitStatus, PtyTest, PtyTestError, Scenario, TestEnv};
use std::time::{Duration, Instant};

fn fixture(label: &str, command: CommandSpec, environment: TestEnv) -> PtyTest {
    PtyTest::spawn(Scenario::new(label).unwrap().command(command).environment(environment)).unwrap()
}

fn command() -> CommandSpec {
    CommandSpec::new(env!("CARGO_BIN_EXE_ptytest-fixture"))
}

#[test]
fn quiescence_waits_the_full_quiet_interval() {
    let mut terminal = fixture("full quiet interval", command(), TestEnv::hermetic().unwrap());
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("PTYTEST_READY")).unwrap();
    let quiet = Duration::from_millis(80);
    let started = Instant::now();
    assert!(terminal.wait_for_quiescence(Deadline::after(Duration::from_secs(2)), quiet).unwrap());
    let elapsed = started.elapsed();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(elapsed >= quiet, "quiet interval ended after {elapsed:?}");
}

#[test]
fn quiescence_returns_false_when_overall_deadline_wins() {
    let mut terminal = fixture("quiet overall deadline", command(), TestEnv::hermetic().unwrap());
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("PTYTEST_READY")).unwrap();
    let quiet = terminal.wait_for_quiescence(Deadline::after(Duration::from_millis(40)), Duration::from_secs(1)).unwrap();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(!quiet);
}

#[test]
fn expired_write_deadline_sends_no_input() {
    let mut terminal = fixture("expired write deadline", command(), TestEnv::hermetic().unwrap());
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("PTYTEST_READY")).unwrap();
    let result = terminal.send_text(Deadline::after(Duration::ZERO), "input\n");
    let wrote = !terminal.raw_input().is_empty();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(matches!(result, Err(PtyTestError::Timeout { .. })), "{result:?}");
    assert!(!wrote);
}

#[test]
fn cleanup_reaps_after_terminal_query_or_trace_failure() {
    for failure in ["query", "overflow"] {
        let environment = TestEnv::hermetic().unwrap();
        let barrier = environment.paths().temp().join("cleanup-output-ready");
        let scenario = Scenario::new(format!("cleanup {failure}")).unwrap()
            .command(command().arg("--cleanup-failure"))
            .environment(environment).trace_limit(96);
        let mut terminal = PtyTest::spawn(scenario).unwrap();
        terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("CLEANUP_READY")).unwrap();
        terminal.send_text(Deadline::after(Duration::from_secs(2)), &format!("{failure}\n")).unwrap();
        let until = Instant::now() + Duration::from_secs(2);
        while !barrier.exists() && Instant::now() < until {
            std::thread::yield_now();
        }
        assert!(barrier.exists(), "fixture never completed its output");
        let result = terminal.finish(Deadline::after(Duration::from_secs(2)));
        let already_reaped = matches!(terminal.signal(0), Err(PtyTestError::ChildAlreadyReaped));
        // Ensure the pre-fix implementation cannot leave the regression child alive.
        if !already_reaped {
            terminal.signal(libc::SIGKILL).unwrap();
            terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
        }
        assert!(already_reaped, "cleanup returned before reaping: {result:?}");
        match failure {
            "query" => assert!(matches!(result, Err(PtyTestError::UnsupportedTerminalQuery { .. }))),
            "overflow" => assert!(matches!(result, Err(PtyTestError::TraceLimitExceeded { .. }))),
            _ => unreachable!(),
        }
        assert_eq!(terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap(), ExitStatus::Signal(libc::SIGKILL));
    }
}

#[test]
fn continuous_output_is_bounded_by_the_read_deadline() {
    let environment = TestEnv::hermetic().unwrap();
    let barrier = environment.paths().temp().join("cleanup-output-ready");
    let mut terminal = fixture("continuous output deadline", command().arg("--cleanup-failure"), environment);
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("CLEANUP_READY")).unwrap();
    terminal.send_text(Deadline::after(Duration::from_secs(2)), "flood\n").unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while !barrier.exists() && Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(barrier.exists());
    let result = terminal.wait_for_screen(Deadline::after(Duration::from_millis(20)), "absent screen", |_| false);
    let bytes_observed = terminal.raw_output().len();
    let activity = terminal.wait_for_output(Deadline::after(Duration::from_millis(20))).unwrap();
    terminal.signal(libc::SIGKILL).unwrap();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(matches!(result, Err(PtyTestError::Timeout { elapsed, .. }) if elapsed >= Duration::from_millis(20)), "{result:?}");
    assert!(bytes_observed > 8192, "continuous output never reached the reader");
    assert!(activity, "output observed before the read timeout was lost");
}

#[test]
fn drop_reaps_after_a_terminal_query_failure() {
    let environment = TestEnv::hermetic().unwrap();
    let barrier = environment.paths().temp().join("cleanup-output-ready");
    let mut terminal = fixture("drop query cleanup", command().arg("--cleanup-failure"), environment);
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("CLEANUP_READY")).unwrap();
    let output = std::str::from_utf8(terminal.raw_output()).unwrap();
    let pid = output.split("CLEANUP_PID=").nth(1).unwrap().split_whitespace().next().unwrap().parse::<libc::pid_t>().unwrap();
    terminal.send_text(Deadline::after(Duration::from_secs(2)), "query\n").unwrap();
    let until = Instant::now() + Duration::from_secs(2);
    while !barrier.exists() && Instant::now() < until {
        std::thread::yield_now();
    }
    assert!(barrier.exists());
    drop(terminal);
    let mut status = 0;
    let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
    let error = std::io::Error::last_os_error().raw_os_error();
    // Keep the regression safe if Drop failed to stop the fixture.
    if result == 0 {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, &mut status, 0);
        }
    }
    assert_eq!(result, -1);
    assert_eq!(error, Some(libc::ECHILD));
}

#[test]
fn zero_quiet_returns_false_after_the_overall_deadline() {
    let mut terminal = fixture("zero quiet expired deadline", command(), TestEnv::hermetic().unwrap());
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("PTYTEST_READY")).unwrap();
    let result = terminal.wait_for_quiescence(Deadline::after(Duration::ZERO), Duration::ZERO);
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(matches!(result, Ok(false)), "{result:?}");
}

#[test]
fn long_quiet_interval_is_capped_by_the_overall_deadline() {
    let mut terminal = fixture("long quiet expired deadline", command(), TestEnv::hermetic().unwrap());
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("PTYTEST_READY")).unwrap();
    let result = terminal.wait_for_quiescence(Deadline::after(Duration::ZERO), Duration::MAX);
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(matches!(result, Ok(false)), "{result:?}");
}
