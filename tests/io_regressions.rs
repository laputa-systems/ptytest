use ptytest::{CommandSpec, Deadline, ExitStatus, PtyTest, PtyTestError, Scenario, TestEnv};
use std::time::{Duration, Instant};

mod support;
use support::fixture_command as command;

fn fixture(label: &str, command: CommandSpec, environment: TestEnv) -> PtyTest {
    PtyTest::spawn(Scenario::new(label).unwrap().command(command).environment(environment)).unwrap()
}

#[test]
fn wait_for_output_distinguishes_idle_from_new_bytes() {
    let mut terminal = fixture("output activity", command(), TestEnv::hermetic().unwrap());
    let ready_until = Instant::now() + Duration::from_secs(2);
    while !terminal.raw_output().ends_with(b"PTYTEST_READY\r\n") {
        assert!(Instant::now() < ready_until, "fixture did not complete its readiness line");
        assert!(terminal.wait_for_output(Deadline::from_instant(ready_until)).unwrap());
    }
    let initial_length = terminal.raw_output().len();
    assert!(!terminal.wait_for_output(Deadline::after(Duration::ZERO)).unwrap());
    let idle = Duration::from_millis(30);
    let started = Instant::now();
    assert!(!terminal.wait_for_output(Deadline::after(idle)).unwrap());
    assert!(started.elapsed() >= idle);
    assert_eq!(terminal.raw_output().len(), initial_length);

    terminal.send_text(Deadline::after(Duration::from_secs(2)), "input\n").unwrap();
    assert!(terminal.wait_for_output(Deadline::after(Duration::from_secs(2))).unwrap());
    assert!(terminal.raw_output().len() > initial_length);
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "input acknowledgment", |s| s.contains("INPUT_ACK")).unwrap();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
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
    let mut terminal = fixture("continuous output deadline", command().arg("--cleanup-failure"), environment);
    terminal.wait_for_screen(Deadline::after(Duration::from_secs(2)), "ready", |s| s.contains("CLEANUP_READY")).unwrap();
    let wait_for_length = |terminal: &mut PtyTest, minimum: usize| {
        let until = Instant::now() + Duration::from_secs(2);
        while terminal.raw_output().len() < minimum && Instant::now() < until {
            terminal.wait_for_output(Deadline::after(Duration::from_millis(25))).unwrap();
        }
        assert!(terminal.raw_output().len() >= minimum, "continuous output never reached the reader");
    };
    let initial_length = terminal.raw_output().len();
    terminal.send_text(Deadline::after(Duration::from_secs(2)), "flood\n").unwrap();
    wait_for_length(&mut terminal, initial_length + 8192);
    let budget = Duration::from_millis(20);
    let result = terminal.wait_for_screen(Deadline::after(budget), "absent screen", |_| false);
    let after_timeout = terminal.raw_output().len();
    wait_for_length(&mut terminal, after_timeout + 1);
    terminal.signal(libc::SIGKILL).unwrap();
    terminal.finish(Deadline::after(Duration::from_secs(2))).unwrap();
    assert!(
        matches!(result, Err(PtyTestError::Timeout { deadline: Some(configured), elapsed, .. })
            if configured == budget && elapsed >= budget),
        "{result:?}"
    );
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
