//! The core's log (F20): Info-level events, plain text, one event per line,
//! English only, in the OS app log directory, rotated at 10 MB × 5 files.
//! Debug mode adds the app's own Debug-level events; other crates stay at
//! Info. Every line passes [`redact`] first, so access and session tokens,
//! OAuth `code`/`state`, `Authorization` headers and Group passwords never
//! reach a log file.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock, RwLock};
use std::time::SystemTime;

use log::{Level, LevelFilter, Log, Metadata, Record};
use regex::{Regex, RegexSet};

/// The current log file; older ones carry `.1` (newest) to `.4`.
pub const FILE_NAME: &str = "exilence-ce-ng.log";
/// A log file is rotated before it would grow beyond this.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Log files kept, the current one included.
pub const FILES: usize = 5;
/// What a secret is replaced with.
pub const REDACTED: &str = "[redacted]";
/// The launch flag that turns Debug mode on.
pub const DEBUG_FLAG: &str = "--debug";
/// The environment variable that turns Debug mode on at launch.
pub const DEBUG_VAR: &str = "EXILENCE_LOG";

/// Starts logging Info-level events into `dir`, for the rest of the process.
pub fn init(dir: &Path) -> io::Result<()> {
    let file = RotatingFile::open(dir)?;
    log::set_boxed_logger(Box::new(Logger {
        file: Mutex::new(file),
    }))
    .map_err(io::Error::other)?;
    let _ = DIR.set(dir.to_owned());
    apply_level();
    Ok(())
}

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// The directory the log is written to, once [`init`] succeeded.
pub fn dir() -> Option<&'static Path> {
    DIR.get().map(PathBuf::as_path)
}

/// Debug mode from the setting.
static DEBUG_SETTING: AtomicBool = AtomicBool::new(false);
/// Debug mode from launch: on for the whole run.
static DEBUG_LAUNCH: AtomicBool = AtomicBool::new(false);

/// Whether `args` (the process's, program name first) or the value of
/// [`DEBUG_VAR`] turn Debug mode on at launch: [`DEBUG_FLAG`] among the
/// arguments, or the variable set to anything but empty, `0`, `false` or
/// `off`.
pub fn debug_at_launch<A: AsRef<OsStr>>(
    args: impl IntoIterator<Item = A>,
    var: Option<&OsStr>,
) -> bool {
    let flag = args.into_iter().skip(1).any(|a| a.as_ref() == DEBUG_FLAG);
    let var = var.is_some_and(|v| {
        let v = v.to_string_lossy();
        let v = v.trim();
        !v.is_empty()
            && !["0", "false", "off"]
                .iter()
                .any(|o| v.eq_ignore_ascii_case(o))
    });
    flag || var
}

/// Turns Debug mode on for the whole run, whatever the setting (F20).
pub fn enable_debug_from_launch() {
    DEBUG_LAUNCH.store(true, Ordering::SeqCst);
    apply_level();
}

/// Whether Debug mode was turned on at launch.
pub fn debug_from_launch() -> bool {
    DEBUG_LAUNCH.load(Ordering::SeqCst)
}

/// Applies the Debug mode setting at once; Debug mode from launch stays on.
pub fn set_debug_mode(on: bool) {
    DEBUG_SETTING.store(on, Ordering::SeqCst);
    apply_level();
}

fn apply_level() {
    let debug = DEBUG_SETTING.load(Ordering::SeqCst) || debug_from_launch();
    log::set_max_level(if debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    });
}

/// Whether an event at `level` from `target` is logged while the level is
/// `max`: up to Info from anyone, Debug from the app only (its crates, the
/// `ninja` crate with the request lines of [`ninja::http::send`], and the
/// UI's `log` command), so other crates' detail never floods the log.
fn wanted(level: Level, target: &str, max: LevelFilter) -> bool {
    let app = target.starts_with("exilence_ce_ng")
        || target == "ninja"
        || target.starts_with("ninja::")
        || target == "ui";
    level <= max && (level <= Level::Info || app)
}

/// Redacts `value` wherever it appears in a later log line: tokens, codes and
/// passwords the core holds, whatever form a line carries them in.
pub fn secret(value: &str) {
    if value.is_empty() {
        return;
    }
    let mut secrets = SECRETS.write().unwrap_or_else(|e| e.into_inner());
    if !secrets.iter().any(|s| s == value) {
        secrets.push(value.to_owned());
        // Longest first, so a secret containing another is replaced whole.
        secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    }
}

static SECRETS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// Names whose values are secret in `name=value` (query, form) and
/// `name: "value"` / `"name":"value"` (Debug, JSON) form.
const SECRET_NAMES: &str = "code|state|code_verifier|access_token|refresh_token|session_token|\
                            ggg_access_token|client_secret|password";

/// The patterns [`redact`] applies, with their replacements.
#[expect(
    clippy::expect_used,
    reason = "the patterns are constants, exercised by the redaction tests"
)]
static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let rule = |pattern: String, replacement| (Regex::new(&pattern).expect("valid"), replacement);
    vec![
        // `Authorization: Bearer x`, `authorization="x"`, `"authorization": "x"`.
        rule(
            r#"(?i)(authorization["']?\s*[:=]\s*["']?)(?:(?:bearer|basic)\s+)?[^\s"',;&]+"#.into(),
            "${1}[redacted]",
        ),
        rule(r"(?i)\b(bearer\s+)[^\s\x22',;&]+".into(), "${1}[redacted]"),
        rule(
            format!(r#"(?i)\b({SECRET_NAMES})=[^\s&"',;]+"#),
            "${1}=[redacted]",
        ),
        rule(
            format!(r#"(?i)\b({SECRET_NAMES})("?\s*:\s*)"(?:[^"\\]|\\.)*""#),
            "${1}${2}\"[redacted]\"",
        ),
    ]
});

#[expect(clippy::expect_used, reason = "built from PATTERNS")]
static ANY_PATTERN: LazyLock<RegexSet> =
    LazyLock::new(|| RegexSet::new(PATTERNS.iter().map(|(r, _)| r.as_str())).expect("valid"));

/// `line` with every secret replaced by [`REDACTED`].
pub fn redact(line: &str) -> Cow<'_, str> {
    let mut line = Cow::Borrowed(line);
    for secret in SECRETS.read().unwrap_or_else(|e| e.into_inner()).iter() {
        if line.contains(secret.as_str()) {
            line = Cow::Owned(line.replace(secret.as_str(), REDACTED));
        }
    }
    if !ANY_PATTERN.is_match(&line) {
        return line;
    }
    let mut owned = line.into_owned();
    for (pattern, replacement) in PATTERNS.iter() {
        if let Cow::Owned(replaced) = pattern.replace_all(&owned, *replacement) {
            owned = replaced;
        }
    }
    Cow::Owned(owned)
}

struct Logger {
    file: Mutex<RotatingFile>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        wanted(metadata.level(), metadata.target(), log::max_level())
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let message = record.args().to_string();
        let line = format!(
            "{} {:<5} {}: {}\n",
            humantime::format_rfc3339_millis(SystemTime::now()),
            record.level(),
            record.target(),
            // One event per line.
            redact(&message).replace('\n', "\\n"),
        );
        let mut file = self.file.lock().unwrap_or_else(|e| e.into_inner());
        // A log that cannot be written has nowhere to report that.
        let _ = file.write(line.as_bytes());
    }

    fn flush(&self) {}
}

/// The name of log file `n`: [`FILE_NAME`] for the current one, then `.1`
/// (newest) to `.4`.
pub fn file_name(n: usize) -> String {
    match n {
        0 => FILE_NAME.to_owned(),
        n => format!("{FILE_NAME}.{n}"),
    }
}

/// [`FILE_NAME`] in a directory, rotated to `.1` … `.4` before it would grow
/// beyond [`MAX_FILE_BYTES`].
pub struct RotatingFile {
    dir: PathBuf,
    file: File,
    len: u64,
}

impl RotatingFile {
    /// Opens the current log file in `dir` for appending, creating both.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let file = Self::open_current(dir)?;
        let len = file.metadata()?.len();
        Ok(Self {
            dir: dir.to_owned(),
            file,
            len,
        })
    }

    fn open_current(dir: &Path) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(FILE_NAME))
    }

    fn path(&self, n: usize) -> PathBuf {
        self.dir.join(file_name(n))
    }

    /// Appends `bytes` (whole lines), rotating first if they would not fit.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.len > 0 && self.len + bytes.len() as u64 > MAX_FILE_BYTES {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.len += bytes.len() as u64;
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        match fs::remove_file(self.path(FILES - 1)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        for n in (0..FILES - 1).rev() {
            match fs::rename(self.path(n), self.path(n + 1)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        self.file = Self::open_current(&self.dir)?;
        self.len = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f20_debug_flag_or_variable_turns_debug_mode_on_at_launch() {
        let var = |v: &'static str| Some(OsStr::new(v));
        assert!(debug_at_launch(["app", "--debug"], None));
        assert!(debug_at_launch(["app"], var("1")));
        assert!(debug_at_launch(["app"], var("debug")));
        assert!(!debug_at_launch(["app"], None));
        assert!(!debug_at_launch(["--debug"], None), "the program name");
        assert!(!debug_at_launch(["app", "--debugger"], None));
        for off in ["", " ", "0", "false", "OFF"] {
            assert!(!debug_at_launch(["app"], var(off)), "{off:?}");
        }
    }

    #[test]
    fn f20_debug_mode_adds_the_apps_own_debug_events_only() {
        let app = "exilence_ce_ng_lib::ggg";
        assert!(wanted(Level::Info, "hyper", LevelFilter::Info));
        assert!(!wanted(Level::Debug, app, LevelFilter::Info));
        assert!(wanted(Level::Debug, app, LevelFilter::Debug));
        assert!(wanted(Level::Debug, "ui", LevelFilter::Debug));
        assert!(wanted(Level::Debug, "ninja::http", LevelFilter::Debug));
        assert!(!wanted(Level::Debug, "ninja_other", LevelFilter::Debug));
        assert!(!wanted(
            Level::Debug,
            "reqwest::connect",
            LevelFilter::Debug
        ));
        assert!(!wanted(Level::Trace, app, LevelFilter::Debug));
    }

    #[test]
    fn f20_patterns_redact_tokens_codes_states_authorization_and_passwords() {
        let cases = [
            (
                "GET /profile Authorization: Bearer abc.def-123",
                "GET /profile Authorization: [redacted]",
            ),
            (
                r#"headers: {"authorization": "Bearer abc"}"#,
                r#"headers: {"authorization": "[redacted]"}"#,
            ),
            ("sent bearer abc123", "sent bearer [redacted]"),
            (
                "exilence-ce-ng://?code=a1b2%2Bc3&state=ng-xyz",
                "exilence-ce-ng://?code=[redacted]&state=[redacted]",
            ),
            (
                r#"SignInRequest { code: "a1", code_verifier: "v\"2" }"#,
                r#"SignInRequest { code: "[redacted]", code_verifier: "[redacted]" }"#,
            ),
            (
                r#"{"access_token":"t1","refresh_token":"t2","expires_in":3600}"#,
                r#"{"access_token":"[redacted]","refresh_token":"[redacted]","expires_in":3600}"#,
            ),
            (
                r#"JoinGroupRequest { name: "Mappers", password: "hunter2" }"#,
                r#"JoinGroupRequest { name: "Mappers", password: "[redacted]" }"#,
            ),
            ("join password=hunter2 now", "join password=[redacted] now"),
            // Not secrets: left alone.
            (
                "GGG answered with status code: 401",
                "GGG answered with status code: 401",
            ),
            ("signed in as Foo#1234", "signed in as Foo#1234"),
        ];
        for (line, expected) in cases {
            assert_eq!(redact(line), expected, "{line}");
        }
    }

    #[test]
    fn f20_registered_secrets_are_redacted_anywhere() {
        secret("s3cr3t-session-token");
        secret("s3cr3t");
        assert_eq!(
            redact("error: token s3cr3t-session-token rejected; s3cr3t"),
            "error: token [redacted] rejected; [redacted]"
        );
    }

    #[test]
    fn f20_log_rotates_at_10_mb_keeping_5_files() {
        assert_eq!((MAX_FILE_BYTES, FILES), (10 * 1024 * 1024, 5));
        let dir = tempfile::tempdir().unwrap();
        let mut file = RotatingFile::open(dir.path()).unwrap();
        let line = |n: usize| format!("{n:08} {}\n", "x".repeat(100 * 1024 - 10));
        let lines = 6 * 1024 * 1024 * 10 / (100 * 1024);
        for n in 0..lines {
            file.write(line(n).as_bytes()).unwrap();
        }

        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "exilence-ce-ng.log",
                "exilence-ce-ng.log.1",
                "exilence-ce-ng.log.2",
                "exilence-ce-ng.log.3",
                "exilence-ce-ng.log.4"
            ]
        );
        for name in &names {
            let len = fs::metadata(dir.path().join(name)).unwrap().len();
            assert!(len <= MAX_FILE_BYTES, "{name}: {len}");
            assert!(
                len > MAX_FILE_BYTES - 200 * 1024 || name == FILE_NAME,
                "{name}: {len}"
            );
        }
        let current = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(current.ends_with(&line(lines - 1)));
        let oldest = fs::read_to_string(dir.path().join("exilence-ce-ng.log.4")).unwrap();
        assert!(!oldest.starts_with(&line(0)), "the oldest file was dropped");

        // Reopening appends to the current file.
        drop(file);
        let mut file = RotatingFile::open(dir.path()).unwrap();
        file.write(b"after restart\n").unwrap();
        let current = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert!(current.ends_with(&format!("{}after restart\n", line(lines - 1))));
    }
}
