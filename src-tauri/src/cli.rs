//! Spawning the ecosystem CLIs.
//!
//! Windows needs two concessions here: `.cmd` shims (pi) only run through the
//! shell, and `CREATE_NO_WINDOW` keeps a console from flashing on every poll.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Default)]
pub struct CliOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CliOutput {
    pub fn ok(&self) -> bool {
        !self.timed_out && self.code == Some(0)
    }

    /// Best available text for logs and error messages.
    pub fn combined(&self) -> String {
        let mut text = self.stdout.trim_end().to_string();
        let stderr = self.stderr.trim_end();
        if !stderr.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(stderr);
        }
        text
    }
}

/// Runs a CLI and captures its output. Never blocks longer than `timeout`.
pub async fn run(program: &str, args: &[String], timeout: Duration) -> Result<CliOutput, String> {
    let home = home_dir();
    run_with_dir(program, args, timeout, home.as_deref()).await
}

/// Same as [`run`] but pinned to a working directory (npm tree checks).
pub async fn run_in(
    program: &str,
    args: &[String],
    timeout: Duration,
    directory: &Path,
) -> Result<CliOutput, String> {
    run_with_dir(program, args, timeout, Some(directory)).await
}

async fn run_with_dir(
    program: &str,
    args: &[String],
    timeout: Duration,
    directory: Option<&Path>,
) -> Result<CliOutput, String> {
    let exe = resolve(program).ok_or_else(|| format!("no se encontró `{program}` en PATH"))?;
    let command_line = display_command(&exe, args);

    let mut command = build(&exe, args);
    if let Some(directory) = directory {
        // `pi list` reports project packages for the cwd; the panel always asks
        // about the user's global install, so pin a stable directory.
        command.current_dir(directory);
    }
    command
        .env("NO_COLOR", "1")
        .env("GENTLE_AI_NO_ANIMATION", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    match tokio::time::timeout(timeout, command.output()).await {
        Err(_) => Ok(CliOutput {
            timed_out: true,
            ..Default::default()
        }),
        Ok(Err(error)) => Err(format!("no se pudo ejecutar {command_line}: {error}")),
        Ok(Ok(output)) => Ok(CliOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            timed_out: false,
        }),
    }
}

/// Runs a CLI and reports each stdout line as it arrives, so long installs
/// show progress instead of a frozen spinner.
pub async fn run_streaming<F>(
    program: &str,
    args: &[String],
    timeout: Duration,
    mut on_line: F,
) -> Result<CliOutput, String>
where
    F: FnMut(&str),
{
    use tokio::io::{AsyncBufReadExt, BufReader};

    let exe = resolve(program).ok_or_else(|| format!("no se encontró `{program}` en PATH"))?;
    let command_line = display_command(&exe, args);

    let mut command = build(&exe, args);
    if let Some(home) = home_dir() {
        command.current_dir(home);
    }
    command
        .env("NO_COLOR", "1")
        .env("GENTLE_AI_NO_ANIMATION", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|error| format!("no se pudo ejecutar {command_line}: {error}"))?;

    let stdout = child.stdout.take().ok_or("stdout no disponible")?;
    let stderr = child.stderr.take().ok_or("stderr no disponible")?;
    let stderr_buffer = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

    let stderr_reader = {
        let buffer = stderr_buffer.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut buffer) = buffer.lock() {
                    buffer.push_str(&line);
                    buffer.push('\n');
                }
            }
        })
    };

    let mut collected = String::new();
    let read = async {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            on_line(&line);
            collected.push_str(&line);
            collected.push('\n');
        }
        child.wait().await
    };

    match tokio::time::timeout(timeout, read).await {
        Err(_) => Ok(CliOutput {
            timed_out: true,
            stdout: collected,
            stderr: stderr_buffer.lock().map(|buffer| buffer.clone()).unwrap_or_default(),
            ..Default::default()
        }),
        Ok(Err(error)) => Err(format!("{command_line}: {error}")),
        Ok(Ok(status)) => {
            let _ = stderr_reader.await;
            Ok(CliOutput {
                code: status.code(),
                stderr: stderr_buffer
                    .lock()
                    .map(|buffer| buffer.clone())
                    .unwrap_or_default(),
                stdout: collected,
                timed_out: false,
            })
        }
    }
}

fn build(exe: &Path, args: &[String]) -> Command {
    let is_exe = exe
        .extension()
        .map(|ext| ext.eq_ignore_ascii_case("exe"))
        .unwrap_or(false);

    if is_exe {
        let mut command = Command::new(exe);
        command.args(args);
        return command;
    }

    // `.cmd`/`.bat` shims need the shell. Rust escapes quotes for
    // CommandLineToArgvW, which cmd.exe does not understand, so the line is
    // appended verbatim: cmd /C ""<exe>" <args>".
    let mut line = format!("\"\"{}\"", exe.display());
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    line.push('"');

    let mut command = Command::new("cmd");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.as_std_mut().raw_arg(format!("/C {line}"));
    }
    #[cfg(not(windows))]
    {
        command.arg("/C").arg(&line);
    }
    command
}

fn display_command(exe: &Path, args: &[String]) -> String {
    let mut text = format!("\"{}\"", exe.display());
    for arg in args {
        text.push(' ');
        text.push_str(&quote(arg));
    }
    text
}

/// Args used by the panel never contain quotes; guard anyway so a package
/// name with spaces cannot break the shell line.
fn quote(arg: &str) -> String {
    let needs_quotes = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || "<>&|^\"()".contains(c));
    if needs_quotes {
        format!("\"{}\"", arg.replace('"', "\"\""))
    } else {
        arg.to_string()
    }
}

/// Resolves a program name against PATH + PATHEXT without spawning `where`.
///
/// Windows PATH often holds extensionless POSIX shims next to the real `.cmd`
/// (`pi` and `pi.cmd` both exist). Only PATHEXT candidates are runnable, so they
/// win; a bare file is never a candidate unless the caller passed an extension.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
}

pub fn resolve(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.is_absolute() || candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    let path = std::env::var_os("PATH")?;
    let extensions: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
        .collect();
    let has_extension = candidate.extension().is_some();

    for directory in std::env::split_paths(&path) {
        if has_extension {
            let with_extension = directory.join(program);
            if with_extension.is_file() {
                return Some(with_extension);
            }
            continue;
        }
        for extension in &extensions {
            let with_extension = directory.join(format!("{program}{extension}"));
            if with_extension.is_file() {
                return Some(with_extension);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_only_when_the_shell_would_split() {
        assert_eq!(quote("npm:gentle-pi"), "npm:gentle-pi");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a&b"), "\"a&b\"");
    }

    #[test]
    fn resolves_known_windows_shims() {
        assert!(resolve("cmd").is_some());
        assert!(resolve("definitely-not-a-real-program-xyz").is_none());
        // `pi` ships as `pi` (POSIX) + `pi.cmd`; the runnable shim must win.
        if let Some(pi) = resolve("pi") {
            let name = pi.file_name().unwrap().to_string_lossy().to_ascii_lowercase();
            assert_eq!(name, "pi.cmd", "resolved {}", pi.display());
        }
    }

    /// Live check against the real machine: `#[ignore]` so the suite stays hermetic.
    /// Run with: cargo test --lib live_pi_shim -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_pi_shim_reports_a_version() {
        let output = run("pi", &["--version".to_string()], Duration::from_secs(60))
            .await
            .expect("pi must be runnable");
        println!("code={:?} stdout={:?} stderr={:?}", output.code, output.stdout, output.stderr);
        assert!(!output.timed_out);
        assert!(
            !output.stdout.trim().is_empty(),
            "pi --version printed nothing; stderr: {}",
            output.stderr
        );
    }

    /// Live check for the ecosystem CLI, which must work without a terminal.
    #[tokio::test]
    #[ignore]
    async fn live_gentle_ai_update_prints_rows() {
        let output = run("gentle-ai", &["update".to_string()], Duration::from_secs(120))
            .await
            .expect("gentle-ai must be runnable");
        println!("code={:?} stdout={:?}", output.code, output.stdout.lines().take(8).collect::<Vec<_>>());
        assert!(
            crate::parsers::parse_gentle_ai_update(&output.stdout).len() >= 2,
            "no rows parsed from: {}",
            output.stdout
        );
    }
}
