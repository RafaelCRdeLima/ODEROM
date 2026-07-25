//! Spawns the real compiled `oderom-repl` binary, feeds it scripted
//! stdin, captures stdout -- exactly `oderom-cli/tests/end_to_end.rs`'s
//! own precedent, and the same reasoning: the REPL's own read loop
//! (rustyline, ctrlc, thread-per-query) is exactly what a purely
//! in-process unit test would skip over, so this is the only test that
//! exercises the whole thing the way a person actually running it does.
//!
//! `HOME` is pointed at a fresh temp dir per test so `~/.oderom_repl_history`
//! never collides between parallel test runs or with a real user's
//! actual history file.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

fn run_repl(script: &str, home_dir: &std::path::Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oderom-repl"))
        .env("HOME", home_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // progress lines go to stderr -- not what these tests check
        .spawn()
        .expect("failed to spawn oderom-repl");
    child.stdin.take().expect("stdin was piped").write_all(script.as_bytes()).expect("failed to write script to stdin");
    let output = child.wait_with_output().expect("failed to wait for oderom-repl");
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Like `run_repl`, but pauses mid-script to run `between` (editing the
/// `.od` file on disk, simulating the user's own editor) *while the
/// child process is still alive* -- `run_repl` alone can't test
/// `:reload` at all, since writing the whole script upfront gives no
/// way to act between two of its lines. Synchronizes on `wait_for`
/// appearing in the child's stdout so far (not a fixed sleep -- this
/// project doesn't guess at timing when it can observe the real
/// condition instead) before running `between` and sending the rest of
/// the script.
fn run_repl_with_edit_midway(before: &str, wait_for: &str, between: impl FnOnce(), after: &str, home_dir: &std::path::Path) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_oderom-repl"))
        .env("HOME", home_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn oderom-repl");

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let captured = Arc::new(Mutex::new(String::new()));
    let reader_captured = captured.clone();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => reader_captured.lock().unwrap().push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });

    stdin.write_all(before.as_bytes()).expect("failed to write script to stdin");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if captured.lock().unwrap().contains(wait_for) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {wait_for:?} in output");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    between();

    stdin.write_all(after.as_bytes()).expect("failed to write rest of script to stdin");
    drop(stdin);

    child.wait().expect("failed to wait for oderom-repl");
    reader.join().expect("reader thread panicked");
    Arc::try_unwrap(captured).unwrap().into_inner().unwrap()
}

const SCHWARZSCHILD: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r),
  [r,r] = 1/(1 - 2*M/r),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

/// The exact cycle the session prompt asked for: load, query, edit the
/// file on disk (simulating the user's own editor -- `:reload`'s whole
/// reason to exist), reload, see the entry marked obsolete, recompute,
/// see it current again -- all inside ONE running REPL process, which
/// is the only way this scenario is real (see
/// `run_repl_with_edit_midway`'s doc comment).
#[test]
fn load_query_edit_reload_obsolete_recompute() {
    let home = tempdir();
    let od_path = home.join("schwarzschild.od");
    std::fs::write(&od_path, SCHWARZSCHILD).unwrap();
    let od_path_str = od_path.display().to_string();

    let before = format!(":load {od_path_str}\nricci\n:entries\n");
    let after = ":reload\n:entries\n:recompute 1\n:entries\n:quit\n";

    let out = run_repl_with_edit_midway(
        &before,
        "[1] ricci -- [OK]",
        || {
            let edited = SCHWARZSCHILD.replace("2*M/r", "3*M/r");
            std::fs::write(&od_path, edited).unwrap();
        },
        after,
        &home,
    );

    // `:reload`/other commands are never echoed to stdout (no terminal,
    // nothing reads them back) -- so the transcript is read as an
    // ordered sequence of `[1] ricci ...` listing lines instead of
    // splitting on command text. There are three: right after load+
    // query (current), right after `:reload` changed the metric
    // (obsolete), and right after `:recompute 1` (current again).
    let entries_lines: Vec<&str> = out.lines().filter(|l| l.starts_with("[1] ricci")).collect();
    assert!(entries_lines.len() >= 3, "expected at least three [1] ricci listings: {out}");
    assert!(!entries_lines[0].contains("OBSOLETE"), "entry should not be stale right after the first query: {out}");
    assert!(
        entries_lines.iter().any(|l| l.contains("OBSOLETE")),
        "entry should have been marked obsolete at some point after :reload changed the metric: {out}"
    );
    assert!(!entries_lines.last().unwrap().contains("OBSOLETE"), "entry should be current again after :recompute: {out}");
}

#[test]
fn defs_lists_declared_names() {
    let home = tempdir();
    let od_path = home.join("schwarzschild.od");
    std::fs::write(&od_path, SCHWARZSCHILD).unwrap();

    let script = format!(":load {}\n:defs\n:quit\n", od_path.display());
    let out = run_repl(&script, &home);
    assert!(out.contains("g"), "{out}");
    assert!(out.contains("schw"), "{out}");
}

#[test]
fn unknown_query_syntax_becomes_a_visible_error_entry() {
    let home = tempdir();
    let od_path = home.join("schwarzschild.od");
    std::fs::write(&od_path, SCHWARZSCHILD).unwrap();

    let script = format!(":load {}\nbogus\n:entries\n:quit\n", od_path.display());
    let out = run_repl(&script, &home);
    assert!(out.contains("ERROR"), "{out}");
}

#[test]
fn recompute_stale_refreshes_every_obsolete_entry() {
    let home = tempdir();
    let od_path = home.join("schwarzschild.od");
    std::fs::write(&od_path, SCHWARZSCHILD).unwrap();
    let od_path_str = od_path.display().to_string();

    let before = format!(":load {od_path_str}\nricci\nchristoffel\n");
    let after = ":reload\n:recompute stale\n:entries\n:quit\n";

    let out = run_repl_with_edit_midway(
        &before,
        "[2] christoffel -- [OK]",
        || {
            let edited = SCHWARZSCHILD.replace("2*M/r", "3*M/r");
            std::fs::write(&od_path, edited).unwrap();
        },
        after,
        &home,
    );

    let after_recompute = out.split_once("recomputed").map(|(_, rest)| rest).expect("expected a 'recomputed N entries' line");
    assert!(!after_recompute.contains("OBSOLETE"), "every entry should be current after :recompute stale: {out}");
}

const REISSNER_NORDSTROM: &str = "
manifold M dim 4
bundle TM on M dim 4
chart schw on M coords (t, r, theta, phi)
metric g on schw bundle TM {
  [t,t] = -(1 - 2*M/r + Q^2/r^2),
  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),
  [theta,theta] = r^2,
  [phi,phi] = r^2 * sin(theta)^2
}
";

/// The scenario the session prompt called out as missing coverage: a
/// SIGINT delivered to the REPL process from *outside* it (not
/// `ReadlineError::Interrupted`, not the in-process `ctx.cancel()` call a
/// unit test could fake) has to (a) cancel the in-flight computation and
/// (b) leave the process -- and the session inside it -- alive and
/// usable afterward. Full real-terminal Ctrl+C behavior needs a real tty
/// and isn't automatable (see the manual pty investigation), but *this*
/// part is: send `kill -INT` at the OS level to the child's actual pid,
/// exactly what a real Ctrl+C keypress ultimately delivers once the
/// terminal driver turns it into a signal.
///
/// Uses RN, not Schwarzschild -- its own fixture comment notes it's slow
/// enough in an unoptimized debug build (this is a `cargo test` binary)
/// to reliably still be mid-computation when the signal lands. Waits for
/// a real stage line on stderr instead of a fixed sleep, matching this
/// file's existing "observe, don't guess at timing" pattern.
#[test]
fn external_sigint_cancels_the_computation_and_the_session_survives() {
    let home = tempdir();
    let od_path = home.join("rn.od");
    std::fs::write(&od_path, REISSNER_NORDSTROM).unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_oderom-repl"))
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn oderom-repl");

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let mut stdout = child.stdout.take().expect("stdout was piped");
    let mut stderr = child.stderr.take().expect("stderr was piped");

    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let stdout_reader = {
        let buf = stdout_buf.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; 4096];
            loop {
                match stdout.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.lock().unwrap().push_str(&String::from_utf8_lossy(&b[..n])),
                }
            }
        })
    };
    let stderr_reader = {
        let buf = stderr_buf.clone();
        std::thread::spawn(move || {
            let mut b = [0u8; 4096];
            loop {
                match stderr.read(&mut b) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.lock().unwrap().push_str(&String::from_utf8_lossy(&b[..n])),
                }
            }
        })
    };

    stdin.write_all(format!(":load {}\nkretschmann\n", od_path.display()).as_bytes()).unwrap();

    // Wait for real evidence the computation has started (a stage line
    // on stderr), not a guessed sleep duration.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if stderr_buf.lock().unwrap().contains("computing Christoffel symbols") {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the computation to start");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // External SIGINT by pid -- the OS-level signal a real Ctrl+C
    // keypress ultimately delivers, sent from outside this process
    // exactly as a real user's shell/terminal driver would, not the
    // in-process `ctx.cancel()` call a unit test could fake.
    let status = Command::new("kill").arg("-INT").arg(child.id().to_string()).status().expect("failed to run kill(1)");
    assert!(status.success(), "kill -INT failed to send the signal");

    stdin.write_all(b":quit\n").unwrap();
    drop(stdin);

    let exit = child.wait().expect("failed to wait for oderom-repl");
    stdout_reader.join().unwrap();
    stderr_reader.join().unwrap();

    assert!(exit.success(), "the REPL process should exit cleanly via :quit after surviving the external SIGINT");

    let out = stdout_buf.lock().unwrap().clone();
    // Etapa 3b gave cancellation its own `EntryState`, shared by
    // `oderom-session`/the REPL/the notebook -- previously this printed
    // "[ERROR] cancelled" (a `Failed` whose message happened to say
    // so), now a genuinely distinct `[CANCELLED]`.
    assert!(out.contains("[1] kretschmann -- [CANCELLED]"), "expected the entry to be marked cancelled: {out}");
}

fn tempdir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("oderom-repl-test-{}-{}", std::process::id(), unique()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Not an RNG dependency for one throwaway unique directory name --
/// nanosecond timestamp plus an in-process counter is unique enough for
/// tests running on one machine.
fn unique() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
    t.wrapping_add(n)
}
