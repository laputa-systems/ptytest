use std::process::Command;

#[test]
fn help_displays_usage_and_exits_successfully() {
    let output = Command::new(env!("CARGO_BIN_EXE_ptytest-replay"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("usage: ptytest-replay"));
    assert!(output.stderr.is_empty());
}
