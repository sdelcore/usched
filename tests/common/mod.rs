//! Shared test scaffolding for CLI integration tests.
//!
//! Each test gets a `Sandbox` that:
//! - allocates a tempdir for HOME and XDG_DATA_HOME
//! - writes stub `systemctl`, `at`, `atrm`, `atq` scripts that exit 0 and
//!   append their argv to a log file
//! - hands back an `assert_cmd::Command` with the stub dir prepended to PATH
//!
//! Stubs are isolated per-test, so tests can run in parallel.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use tempfile::TempDir;

pub struct Sandbox {
    pub root: TempDir,
}

impl Sandbox {
    pub fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let sandbox = Sandbox { root };
        sandbox.write_stubs();
        sandbox
    }

    pub fn home(&self) -> PathBuf {
        self.root.path().join("home")
    }

    pub fn xdg_data(&self) -> PathBuf {
        // dirs::data_local_dir() honors XDG_DATA_HOME; usched appends "usched/"
        self.home().join(".local/share")
    }

    pub fn data_dir(&self) -> PathBuf {
        self.xdg_data().join("usched")
    }

    pub fn stub_bin(&self) -> PathBuf {
        self.root.path().join("stubs")
    }

    pub fn stub_log(&self) -> PathBuf {
        self.root.path().join("stub.log")
    }

    fn write_stubs(&self) {
        let bin = self.stub_bin();
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(self.home()).unwrap();
        fs::create_dir_all(self.xdg_data()).unwrap();

        let log = self.stub_log();
        // Each stub records "<name> <args>" then exits 0. The `at` stub also
        // emits a fake "job N" line on stderr so usched can parse a job number.
        write_stub(&bin, "systemctl", &log, None);
        write_stub(&bin, "atrm", &log, None);
        write_stub(&bin, "atq", &log, None);
        write_stub(
            &bin,
            "at",
            &log,
            Some("warning: stubbed at\njob 42 at Thu Jan 16 14:00:00 2099"),
        );
    }

    /// Read the stub invocation log, splitting into one entry per call.
    pub fn invocations(&self) -> Vec<String> {
        match fs::read_to_string(self.stub_log()) {
            Ok(s) => s.lines().map(|l| l.to_string()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn invocations_for(&self, prog: &str) -> Vec<String> {
        self.invocations()
            .into_iter()
            .filter(|l| l.starts_with(&format!("{} ", prog)) || l == prog)
            .collect()
    }

    /// Overwrite a stub with a custom bash script body. The script still has
    /// the sandbox PATH on it. Useful for simulating failure modes (e.g.
    /// `atrm` failing with the at(1) "Cannot get uid for atd" error).
    pub fn override_stub(&self, name: &str, script_body: &str) {
        let path = self.stub_bin().join(name);
        let log = self.stub_log();
        let log_q = shell_quote(&log.to_string_lossy());
        let script = format!(
            "#!/usr/bin/env bash\nprintf '%s' {prog} >> {log}\nfor a in \"$@\"; do printf ' %s' \"$a\" >> {log}; done\nprintf '\\n' >> {log}\n{body}\n",
            prog = name,
            log = log_q,
            body = script_body,
        );
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    /// Build an `assert_cmd::Command` for the usched binary with sandbox env.
    pub fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("usched").expect("usched binary");
        let original_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", self.stub_bin().display(), original_path);

        // env_clear() would lose system PATH entries we still need (bash, which).
        // Instead: override the few vars usched cares about.
        cmd.env("HOME", self.home())
            .env("XDG_DATA_HOME", self.xdg_data())
            .env("PATH", new_path);
        cmd
    }

    pub fn jobs_json_path(&self) -> PathBuf {
        self.data_dir().join("jobs.json")
    }

    pub fn state_json_path(&self) -> PathBuf {
        self.data_dir().join("state.json")
    }

    pub fn systemd_user_dir(&self) -> PathBuf {
        self.home().join(".config/systemd/user")
    }
}

fn write_stub(bin_dir: &Path, name: &str, log: &Path, stderr: Option<&str>) {
    let path = bin_dir.join(name);
    let stderr_line = stderr
        .map(|s| format!("printf '%s\\n' {} 1>&2\n", shell_quote(s)))
        .unwrap_or_default();
    let script = format!(
        "#!/usr/bin/env bash\nprintf '%s' {prog} >> {log}\nfor a in \"$@\"; do printf ' %s' \"$a\" >> {log}; done\nprintf '\\n' >> {log}\n{stderr}exit 0\n",
        prog = name,
        log = shell_quote(&log.to_string_lossy()),
        stderr = stderr_line,
    );
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
