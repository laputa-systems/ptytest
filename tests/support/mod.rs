use ptytest::CommandSpec;

pub fn fixture_command() -> CommandSpec {
    let command = CommandSpec::new(env!("CARGO_BIN_EXE_ptytest-fixture"));
    match std::env::var_os("LLVM_PROFILE_FILE") {
        Some(profile) => command.env("LLVM_PROFILE_FILE", profile),
        None => command,
    }
}
