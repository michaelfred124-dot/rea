//! Updating the app in place from its source checkout: pull the newest code,
//! build it, swap the new programs in next to the running one and restart.
//!
//! The checkout is named by `hive-rig-source.txt` next to the program (one
//! line: the repository folder, or its `hive-rig` folder), or by the
//! `HIVE_RIG_SOURCE` environment variable. The work runs on its own thread so
//! the window keeps drawing; [`Updater::poll`] picks up its progress.
//!
//! Windows won't overwrite a running program but will rename it, so the old
//! programs become `*.old.exe` and are deleted on the next start.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};

/// The programs swapped on an update.
const PROGRAMS: [&str; 2] = ["HiveRig", "hive-rig"];
/// The file next to the program naming the source checkout.
pub const SOURCE_FILE: &str = "hive-rig-source.txt";

/// Where an update stands.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateState {
    Idle,
    /// Running; what it is doing.
    Working(String),
    /// Nothing newer than what is running.
    UpToDate(String),
    /// A new version is in place; restart to use it.
    Ready(String),
    Failed(String),
}

enum Step {
    Progress(String),
    Done(UpdateState),
}

pub struct Updater {
    pub state: UpdateState,
    rx: Option<Receiver<Step>>,
}

impl Default for Updater {
    fn default() -> Self {
        Updater {
            state: UpdateState::Idle,
            rx: None,
        }
    }
}

impl Updater {
    pub fn busy(&self) -> bool {
        matches!(self.state, UpdateState::Working(_))
    }

    /// Start updating, unless an update is already running.
    pub fn start(&mut self) {
        if self.busy() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        self.state = UpdateState::Working("Looking for the source…".into());
        std::thread::spawn(move || {
            let report = |s: &str| {
                let _ = tx.send(Step::Progress(s.to_string()));
            };
            let end = match run(&report) {
                Ok(state) => state,
                Err(e) => UpdateState::Failed(e),
            };
            let _ = tx.send(Step::Done(end));
        });
    }

    /// Take in the update's progress. Returns true when its state changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else {
            return false;
        };
        let mut changed = false;
        while let Ok(step) = rx.try_recv() {
            changed = true;
            match step {
                Step::Progress(s) => self.state = UpdateState::Working(s),
                Step::Done(s) => {
                    self.state = s;
                    self.rx = None;
                    break;
                }
            }
        }
        changed
    }
}

/// The folder the running program is in.
fn program_dir() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Couldn't find the program: {e}"))?;
    exe.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "Couldn't find the program's folder.".into())
}

/// The source checkout's `hive-rig` folder (the one with Cargo.toml).
pub fn source_dir(program_dir: &Path) -> Result<PathBuf, String> {
    let named = match std::fs::read_to_string(program_dir.join(SOURCE_FILE)) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .map(PathBuf::from),
        Err(_) => std::env::var_os("HIVE_RIG_SOURCE").map(PathBuf::from),
    };
    let Some(dir) = named else {
        return Err(format!(
            "Updating needs the Hive Rig source: put its folder in {SOURCE_FILE} next to HiveRig.exe."
        ));
    };
    [dir.join("hive-rig"), dir.clone()]
        .into_iter()
        .find(|d| d.join("Cargo.toml").is_file() && d.join("src").is_dir())
        .ok_or_else(|| format!("{} isn't a Hive Rig source folder.", dir.display()))
}

/// Where builds go: outside the checkout, so a synced folder stays small.
fn target_dir() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(d) => PathBuf::from(d).join("HiveRig").join("target"),
        None => std::env::temp_dir().join("HiveRig").join("target"),
    }
}

/// A command with no console window popping up behind the app.
fn quiet(program: &str) -> Command {
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Run a command; its output's last lines are the error when it fails.
fn output(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("Couldn't run {what}: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        return Ok(text);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let tail: Vec<&str> = err.lines().rev().take(4).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    Err(format!("{what} failed: {}", tail.join(" ")))
}

fn version(src: &Path) -> String {
    output(
        quiet("git")
            .arg("-C")
            .arg(src)
            .args(["log", "-1", "--format=%h %s"]),
        "git",
    )
    .unwrap_or_else(|_| "unknown version".into())
}

fn run(report: &dyn Fn(&str)) -> Result<UpdateState, String> {
    run_in(&program_dir()?, report)
}

/// [`run`] for the programs in `dir`.
fn run_in(dir: &Path, report: &dyn Fn(&str)) -> Result<UpdateState, String> {
    let dir = dir.to_path_buf();
    let src = source_dir(&dir)?;
    report("Getting the newest Hive Rig…");
    let before = version(&src);
    let pulled = output(
        quiet("git")
            .arg("-C")
            .arg(&src)
            .args(["pull", "--ff-only", "-q"]),
        "Getting the newest code (git pull)",
    );
    let after = version(&src);
    let target = target_dir();
    let built = |name: &str| target.join("release").join(format!("{name}.exe"));
    // Nothing new, and the programs here are that build already.
    let same_build = PROGRAMS.iter().all(|p| {
        let (a, b) = (built(p), dir.join(format!("{p}.exe")));
        matches!((std::fs::metadata(&a), std::fs::metadata(&b)),
            (Ok(x), Ok(y)) if x.len() == y.len()
                && x.modified().ok() <= y.modified().ok())
    });
    if pulled.is_ok() && before == after && same_build {
        return Ok(UpdateState::UpToDate(after));
    }
    report("Building the new version (about a minute)…");
    output(
        quiet("cargo")
            .current_dir(&src)
            .env("CARGO_TARGET_DIR", &target)
            .args(["build", "--release", "--bins"]),
        "Building (cargo build)",
    )?;
    report("Putting the new version in place…");
    for p in PROGRAMS {
        let new = built(p);
        let here = dir.join(format!("{p}.exe"));
        let old = dir.join(format!("{p}.old.exe"));
        let _ = std::fs::remove_file(&old);
        if here.exists() {
            std::fs::rename(&here, &old)
                .map_err(|e| format!("Couldn't move the old {p}.exe aside: {e}"))?;
        }
        if let Err(e) = std::fs::copy(&new, &here) {
            let _ = std::fs::rename(&old, &here);
            return Err(format!("Couldn't copy the new {p}.exe: {e}"));
        }
    }
    let note = match pulled {
        Ok(_) => after,
        Err(_) => format!("{after} (couldn't reach GitHub, built what was there)"),
    };
    Ok(UpdateState::Ready(note))
}

/// Delete the programs a finished update moved aside.
pub fn clean_up() {
    if let Ok(dir) = program_dir() {
        for p in PROGRAMS {
            let _ = std::fs::remove_file(dir.join(format!("{p}.old.exe")));
        }
    }
}

/// Start the new program (with no arguments) for a restart.
pub fn relaunch() -> Result<(), String> {
    let dir = program_dir()?;
    Command::new(dir.join("HiveRig.exe"))
        .current_dir(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Couldn't start the new Hive Rig: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_file_names_the_checkout() {
        let tmp = std::env::temp_dir().join(format!("hive_update_{}", std::process::id()));
        let repo = tmp.join("repo");
        let crate_dir = repo.join("hive-rig");
        std::fs::create_dir_all(crate_dir.join("src")).unwrap();
        std::fs::write(crate_dir.join("Cargo.toml"), "[package]").unwrap();
        let program = tmp.join("app");
        std::fs::create_dir_all(&program).unwrap();
        // The repository folder or its hive-rig folder both work.
        for named in [&repo, &crate_dir] {
            std::fs::write(
                program.join(SOURCE_FILE),
                format!("# Hive Rig source\n{}\n", named.display()),
            )
            .unwrap();
            assert_eq!(source_dir(&program).unwrap(), crate_dir);
        }
        std::fs::write(
            program.join(SOURCE_FILE),
            tmp.join("nowhere").display().to_string(),
        )
        .unwrap();
        assert!(
            source_dir(&program)
                .unwrap_err()
                .contains("isn't a Hive Rig source")
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn progress_reaches_the_window() {
        let mut u = Updater::default();
        let (tx, rx) = channel();
        u.rx = Some(rx);
        u.state = UpdateState::Working("start".into());
        tx.send(Step::Progress("building".into())).unwrap();
        assert!(u.poll());
        assert_eq!(u.state, UpdateState::Working("building".into()));
        tx.send(Step::Done(UpdateState::Ready("abc".into())))
            .unwrap();
        assert!(u.poll());
        assert_eq!(u.state, UpdateState::Ready("abc".into()));
        assert!(!u.busy() && !u.poll());
    }

    /// The whole update on copies of the programs: pull, build, swap. Slow (it
    /// builds), so run it by hand: cargo test --release --bin HiveRig -- --ignored
    #[test]
    #[ignore]
    fn updates_a_copy_of_the_programs() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = std::env::temp_dir().join(format!("hive_update_run_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for p in PROGRAMS {
            std::fs::write(dir.join(format!("{p}.exe")), b"old").unwrap();
        }
        std::fs::write(dir.join(SOURCE_FILE), src.display().to_string()).unwrap();
        let state = run_in(&dir, &|s| println!("{s}")).unwrap();
        assert!(matches!(state, UpdateState::Ready(_)), "{state:?}");
        for p in PROGRAMS {
            let new = std::fs::metadata(dir.join(format!("{p}.exe")))
                .unwrap()
                .len();
            assert!(new > 1_000_000, "{p}.exe is {new} bytes");
            assert_eq!(
                std::fs::read(dir.join(format!("{p}.old.exe"))).unwrap(),
                b"old"
            );
        }
        // Run again: nothing new, so it says so and leaves the programs be.
        let again = run_in(&dir, &|_| {}).unwrap();
        assert!(matches!(again, UpdateState::UpToDate(_)), "{again:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
