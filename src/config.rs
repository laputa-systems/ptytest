use crate::{PtyTestError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A non-zero terminal size in terminal order: columns, then rows.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Size {
    columns: u16,
    rows: u16,
}

impl Size {
    pub fn new(columns: u16, rows: u16) -> Result<Self> {
        if columns == 0 || rows == 0 {
            return Err(PtyTestError::InvalidSize { columns, rows });
        }
        Ok(Self { columns, rows })
    }

    pub const fn columns(self) -> u16 {
        self.columns
    }

    pub const fn rows(self) -> u16 {
        self.rows
    }
}

/// A caller-supplied monotonic deadline.
#[derive(Clone, Copy, Debug)]
pub struct Deadline {
    instant: Instant,
    configured_for: Option<Duration>,
}

impl Deadline {
    pub fn after(duration: Duration) -> Self {
        Self {
            instant: Instant::now() + duration,
            configured_for: Some(duration),
        }
    }

    pub fn from_instant(instant: Instant) -> Self {
        Self {
            instant,
            configured_for: None,
        }
    }

    pub(crate) fn remaining(self) -> Option<Duration> {
        self.instant.checked_duration_since(Instant::now())
    }

    pub(crate) fn expired(self) -> bool {
        self.remaining().is_none()
    }

    pub(crate) fn configured_for(self) -> Option<Duration> {
        self.configured_for
    }
}

/// A directly executed program and its explicitly owned process settings.
#[derive(Clone, Debug)]
pub struct CommandSpec {
    program: PathBuf,
    arguments: Vec<OsString>,
    current_dir: Option<PathBuf>,
    environment: BTreeMap<OsString, Option<OsString>>,
    secret_environment: BTreeSet<OsString>,
}

impl CommandSpec {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            current_dir: None,
            environment: BTreeMap::new(),
            secret_environment: BTreeSet::new(),
        }
    }

    pub fn arg(mut self, argument: impl AsRef<OsStr>) -> Self {
        self.arguments.push(argument.as_ref().to_os_string());
        self
    }

    pub fn args<I, S>(mut self, arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.arguments.extend(
            arguments
                .into_iter()
                .map(|argument| argument.as_ref().to_os_string()),
        );
        self
    }

    pub fn current_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(path.into());
        self
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.environment.insert(
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        );
        self
    }

    pub fn remove_env(mut self, key: impl AsRef<OsStr>) -> Self {
        self.environment.insert(key.as_ref().to_os_string(), None);
        self
    }

    /// Adds an environment value while marking it for textual-diagnostic
    /// redaction. Raw input/output are intentionally never redacted.
    pub fn secret_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.secret_environment.insert(key.as_ref().to_os_string());
        self.environment.insert(
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        );
        self
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    pub fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    pub fn current_dir_path(&self) -> Option<&Path> {
        self.current_dir.as_deref()
    }

    pub(crate) fn environment(&self) -> &BTreeMap<OsString, Option<OsString>> {
        &self.environment
    }

    pub(crate) fn is_secret_environment(&self, key: &OsStr) -> bool {
        self.secret_environment.contains(key)
    }
}

/// Paths created for a hermetic test environment.
#[derive(Clone, Debug)]
pub struct TestPaths {
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
    data: PathBuf,
    cache: PathBuf,
    state: PathBuf,
    temp: PathBuf,
    fixtures: PathBuf,
}

impl TestPaths {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn config(&self) -> &Path {
        &self.config
    }
    pub fn data(&self) -> &Path {
        &self.data
    }
    pub fn cache(&self) -> &Path {
        &self.cache
    }
    pub fn state(&self) -> &Path {
        &self.state
    }
    pub fn temp(&self) -> &Path {
        &self.temp
    }
    pub fn fixtures(&self) -> &Path {
        &self.fixtures
    }
}

/// An environment that does not inherit application settings from the test
/// runner. Its scratch directory is removed when the environment is dropped.
#[derive(Debug)]
pub struct TestEnv {
    variables: BTreeMap<OsString, Option<OsString>>,
    secret_variables: BTreeSet<OsString>,
    paths: TestPaths,
    keep_scratch: bool,
}

impl TestEnv {
    /// The default hermetic environment, with the portable ASCII locale.
    pub fn hermetic() -> Result<Self> {
        Self::hermetic_ascii()
    }

    pub fn hermetic_ascii() -> Result<Self> {
        Self::new_with_locale("C")
    }

    /// Creates an isolated environment with a caller-selected, already
    /// validated UTF-8 locale. Locale availability is checked on the host so
    /// a test cannot silently fall back to a non-UTF-8 environment.
    pub fn hermetic_utf8(locale: impl Into<String>) -> Result<Self> {
        let locale = locale.into();
        if !locale_has_utf8(&locale) {
            return Err(PtyTestError::Utf8LocaleUnavailable { locale });
        }
        Self::new_with_locale(&locale)
    }

    fn new_with_locale(locale: &str) -> Result<Self> {
        let paths = create_scratch_paths()?;
        let mut variables = BTreeMap::new();
        insert_env_path(&mut variables, "HOME", paths.home());
        insert_env_path(&mut variables, "XDG_CONFIG_HOME", paths.config());
        insert_env_path(&mut variables, "XDG_DATA_HOME", paths.data());
        insert_env_path(&mut variables, "XDG_CACHE_HOME", paths.cache());
        insert_env_path(&mut variables, "XDG_STATE_HOME", paths.state());
        insert_env_path(&mut variables, "TMPDIR", paths.temp());
        variables.insert(
            OsString::from("TERM"),
            Some(OsString::from("xterm-256color")),
        );
        variables.insert(
            OsString::from("PATH"),
            Some(OsString::from("/usr/bin:/bin")),
        );
        variables.insert(OsString::from("LANG"), Some(OsString::from(locale)));
        variables.insert(OsString::from("LC_ALL"), Some(OsString::from(locale)));
        variables.insert(OsString::from("TZ"), Some(OsString::from("UTC")));
        Ok(Self {
            variables,
            secret_variables: BTreeSet::new(),
            paths,
            keep_scratch: false,
        })
    }

    pub fn env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.variables.insert(
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        );
        self
    }

    pub fn remove_env(mut self, key: impl AsRef<OsStr>) -> Self {
        self.variables.insert(key.as_ref().to_os_string(), None);
        self
    }

    /// Adds an environment value while marking it for textual-diagnostic
    /// redaction. Raw input/output are intentionally never redacted.
    pub fn secret_env(mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.secret_variables.insert(key.as_ref().to_os_string());
        self.variables.insert(
            key.as_ref().to_os_string(),
            Some(value.as_ref().to_os_string()),
        );
        self
    }

    /// Preserves the scratch root after drop for a caller that needs to inspect
    /// fixture-created files. Failure artifacts are independent of this flag.
    pub fn keep_scratch(mut self) -> Self {
        self.keep_scratch = true;
        self
    }

    pub fn paths(&self) -> &TestPaths {
        &self.paths
    }

    pub(crate) fn variables(&self) -> &BTreeMap<OsString, Option<OsString>> {
        &self.variables
    }

    pub(crate) fn is_secret_variable(&self, key: &OsStr) -> bool {
        self.secret_variables.contains(key)
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if !self.keep_scratch {
            let _ = fs::remove_dir_all(self.paths.root());
        }
    }
}

/// A named, reproducible PTY test configuration.
#[derive(Debug)]
pub struct Scenario {
    label: String,
    artifact_label: String,
    command: Option<CommandSpec>,
    size: Size,
    environment: Option<TestEnv>,
    trace_limit: usize,
    protocol_profile: crate::ProtocolProfile,
}

impl Scenario {
    pub fn new(label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let artifact_label = sanitize_label(&label)?;
        Ok(Self {
            label,
            artifact_label,
            command: None,
            size: Size::new(80, 24).expect("constant terminal size is non-zero"),
            environment: None,
            trace_limit: 8 * 1024 * 1024,
            protocol_profile: crate::ProtocolProfile::disabled(),
        })
    }

    pub fn command(mut self, command: CommandSpec) -> Self {
        self.command = Some(command);
        self
    }

    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    pub fn environment(mut self, environment: TestEnv) -> Self {
        self.environment = Some(environment);
        self
    }

    pub fn trace_limit(mut self, bytes: usize) -> Self {
        self.trace_limit = bytes;
        self
    }

    pub fn protocol_profile(mut self, profile: crate::ProtocolProfile) -> Self {
        self.protocol_profile = profile;
        self
    }

    pub fn label(&self) -> &str {
        &self.label
    }
    pub fn size_value(&self) -> Size {
        self.size
    }
    pub fn artifact_label(&self) -> &str {
        &self.artifact_label
    }
    pub fn environment_paths(&self) -> Option<&TestPaths> {
        self.environment.as_ref().map(TestEnv::paths)
    }

    pub(crate) fn into_parts(self) -> Result<ScenarioParts> {
        let command = self.command.ok_or(PtyTestError::MissingScenarioCommand)?;
        let environment = match self.environment {
            Some(environment) => environment,
            None => TestEnv::hermetic()?,
        };
        Ok(ScenarioParts {
            label: self.label,
            artifact_label: self.artifact_label,
            command,
            size: self.size,
            environment,
            trace_limit: self.trace_limit,
            protocol_profile: self.protocol_profile,
        })
    }
}

pub(crate) struct ScenarioParts {
    pub(crate) label: String,
    pub(crate) artifact_label: String,
    pub(crate) command: CommandSpec,
    pub(crate) size: Size,
    pub(crate) environment: TestEnv,
    pub(crate) trace_limit: usize,
    pub(crate) protocol_profile: crate::ProtocolProfile,
}

impl ScenarioParts {
    pub(crate) fn command_description(&self) -> String {
        let mut output = format!("program: {}\n", self.command.program().display());
        for argument in self.command.arguments() {
            output.push_str(&format!("arg: {}\n", argument.to_string_lossy()));
        }
        if let Some(path) = self.command.current_dir_path() {
            output.push_str(&format!("cwd: {}\n", path.display()));
        }
        let mut variables = self.environment.variables().clone();
        variables.extend(
            self.command
                .environment()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        for (key, value) in variables {
            let key_text = key.to_string_lossy();
            let secret = self.environment.is_secret_variable(&key)
                || self.command.is_secret_environment(&key)
                || looks_secret(&key_text);
            let value = match value {
                Some(_) if secret => "<redacted>".into(),
                Some(value) => value.to_string_lossy().into_owned(),
                None => "<removed>".into(),
            };
            output.push_str(&format!("env {key_text}={value}\n"));
        }
        output
    }
}

fn looks_secret(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "credential",
        "api_key",
        "apikey",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn insert_env_path(
    environment: &mut BTreeMap<OsString, Option<OsString>>,
    key: &str,
    value: &Path,
) {
    environment.insert(key.into(), Some(value.as_os_str().to_os_string()));
}

fn create_scratch_paths() -> Result<TestPaths> {
    let parent = std::env::temp_dir().join("ptytest");
    fs::create_dir_all(&parent)
        .map_err(|error| PtyTestError::io_at("create scratch parent", &parent, error))?;

    let pid = std::process::id();
    let root = loop {
        let sequence = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = parent.join(format!("{pid}-{sequence}"));
        match fs::create_dir(&candidate) {
            Ok(()) => break candidate,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(PtyTestError::io_at("create scratch root", candidate, error)),
        }
    };
    let paths = TestPaths {
        home: root.join("home"),
        config: root.join("config"),
        data: root.join("data"),
        cache: root.join("cache"),
        state: root.join("state"),
        temp: root.join("tmp"),
        fixtures: root.join("fixtures"),
        root,
    };
    for path in [
        paths.home(),
        paths.config(),
        paths.data(),
        paths.cache(),
        paths.state(),
        paths.temp(),
        paths.fixtures(),
    ] {
        fs::create_dir(path)
            .map_err(|error| PtyTestError::io_at("create scratch directory", path, error))?;
    }
    Ok(paths)
}

fn locale_has_utf8(locale: &str) -> bool {
    let Ok(output) = std::process::Command::new("/usr/bin/locale")
        .arg("-a")
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|available| normalize_locale_name(available) == normalize_locale_name(locale))
}

// Darwin lists `C.UTF-8`, while glibc commonly lists the equivalent
// `C.utf8`. The locale supplied to the child remains the caller's spelling;
// this normalization only keeps host-side availability checking portable.
fn normalize_locale_name(locale: &str) -> String {
    locale
        .chars()
        .filter(|character| *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn sanitize_label(label: &str) -> Result<String> {
    if label.trim().is_empty() {
        return Err(PtyTestError::InvalidScenarioLabel {
            label: label.to_owned(),
            reason: "label is empty",
        });
    }
    let mut output = String::with_capacity(label.len());
    let mut previous_dash = false;
    for character in label.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
            output.push(character);
            previous_dash = false;
        } else if character.is_ascii_whitespace() {
            if !previous_dash {
                output.push('-');
                previous_dash = true;
            }
        } else {
            return Err(PtyTestError::InvalidScenarioLabel {
                label: label.to_owned(),
                reason: "only ASCII letters, digits, dot, underscore, dash, and spaces are allowed",
            });
        }
    }
    let output = output.trim_matches(['-', '.']).to_owned();
    if output.is_empty() || output == "." || output == ".." {
        return Err(PtyTestError::InvalidScenarioLabel {
            label: label.to_owned(),
            reason: "label has no usable path component",
        });
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_rejects_zero_dimensions() {
        assert!(matches!(
            Size::new(0, 24),
            Err(PtyTestError::InvalidSize { .. })
        ));
        assert!(matches!(
            Size::new(80, 0),
            Err(PtyTestError::InvalidSize { .. })
        ));
        assert_eq!(Size::new(80, 24).unwrap().columns(), 80);
    }

    #[test]
    fn scenario_label_is_safe_and_stable_for_artifact_paths() {
        let scenario = Scenario::new("startup screen 1").unwrap();
        assert_eq!(scenario.label(), "startup screen 1");
        assert_eq!(scenario.artifact_label(), "startup-screen-1");
        assert!(Scenario::new("../escape").is_err());
        assert!(Scenario::new("   ").is_err());
    }

    #[test]
    fn hermetic_environment_clears_inheritance_and_allows_explicit_overrides() {
        let environment = TestEnv::hermetic_ascii()
            .unwrap()
            .env("TERM", "screen-256color")
            .remove_env("TZ");
        assert_eq!(
            environment.variables().get(OsStr::new("TERM")),
            Some(&Some(OsString::from("screen-256color")))
        );
        assert_eq!(environment.variables().get(OsStr::new("TZ")), Some(&None));
        assert!(!environment.variables().contains_key(OsStr::new("USER")));
        assert!(environment.paths().fixtures().is_dir());
    }

    #[test]
    fn parallel_scratch_roots_are_unique() {
        let first = TestEnv::hermetic().unwrap().keep_scratch();
        let second = TestEnv::hermetic().unwrap().keep_scratch();
        assert_ne!(first.paths().root(), second.paths().root());
        let first_root = first.paths().root().to_owned();
        let second_root = second.paths().root().to_owned();
        drop(first);
        drop(second);
        fs::remove_dir_all(first_root).unwrap();
        fs::remove_dir_all(second_root).unwrap();
    }

    #[test]
    fn unavailable_utf8_locale_fails_explicitly() {
        assert!(matches!(
            TestEnv::hermetic_utf8("ptytest-impossible.UTF-8"),
            Err(PtyTestError::Utf8LocaleUnavailable { .. })
        ));
    }

    #[test]
    fn c_utf8_is_a_supported_cross_platform_unicode_environment() {
        let environment = TestEnv::hermetic_utf8("C.UTF-8").unwrap();
        assert_eq!(
            environment.variables().get(OsStr::new("LANG")),
            Some(&Some(OsString::from("C.UTF-8")))
        );
        assert_eq!(
            environment.variables().get(OsStr::new("LC_ALL")),
            Some(&Some(OsString::from("C.UTF-8")))
        );
    }
}
