//! ODEROM terminal REPL -- ETAPA 2 of DESIGN-UI-SESSION.md. Proves
//! `oderom-session` end to end against a real human before any window
//! exists: load definitions, run queries, edit the `.od` file in a real
//! editor, `:reload`, see obsolescence, recompute. No color, no
//! autocomplete, no multi-line editing -- see the module's own command
//! table for exactly what's in scope.
//!
//! # Ctrl+C
//!
//! Cancels the computation in flight, not the REPL (`ctrlc`, the one
//! new dependency besides `rustyline`'s line history -- overriding the
//! default "SIGINT kills the process" behavior needs a signal handler,
//! and this project forbids `unsafe_code`, so a hand-rolled one isn't an
//! option). `current_ctx` below is the shared slot the handler and the
//! main loop use to agree on "is anything running right now": empty
//! while at the prompt (where Ctrl+C is rustyline's own concern --
//! interrupts the current input line, standard readline behavior, not
//! this handler's), holding the in-flight query's `ExecutionContext`
//! while a computation runs on its worker thread.
//!
//! # Timeout
//!
//! `:timeout <seconds>` sets a wall-clock budget (default 30s, same as
//! the CLI's own `--timeout`); a query that outruns it is cancelled the
//! same way Ctrl+C cancels one (`ExecutionContext::cancel`), not killed
//! by force -- see `Repl::recv_with_timeout`.

use oderom_cli::commands::ExecutionContext;
use oderom_cli::CliError;
use oderom_session::{Entry, EntryId, EntryState, Session};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Same default as the CLI's own `--timeout` (`oderom-cli/src/commands.rs`'s
/// `DEFAULT_TIMEOUT`, kept private there) -- a wall-clock budget the CLI
/// has always had but the REPL never applied at all: `run_query`
/// (`oderom-session`) has no timeout of its own, and `cmd_query`/
/// `recompute_one` below used to block on `rx.recv()` with no bound.
/// Found the hard way (a metric whose `g_tt`/`g_rr` are not reciprocal
/// ran for 60+s with no guardrail firing) -- adjustable per-session via
/// `:timeout <seconds>`, not just this fixed default.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// `$HOME/.oderom_repl_history` -- falls back to the bare filename (in
/// whatever the current directory happens to be) only if `$HOME` isn't
/// set at all, which real interactive use never hits. Reading `$HOME`
/// here (not a fixed relative path) is also what lets a test isolate
/// its own history file by setting `$HOME` to a temp dir, instead of
/// every parallel test run colliding on one file in the working
/// directory.
fn history_path() -> std::path::PathBuf {
    match std::env::var_os("HOME") {
        Some(home) => std::path::Path::new(&home).join(".oderom_repl_history"),
        None => std::path::PathBuf::from(".oderom_repl_history"),
    }
}

fn main() {
    // `oderom_expr::run_cancellable` (armed for the whole query duration
    // by `oderom-session::run_query`, so a cancellation discovered deep
    // inside a single component's `normalize()` call can still unwind
    // out) uses a panic under the hood -- see that function's own doc
    // comment for why. Rust's default panic hook still prints to stderr
    // for a panic `catch_unwind` goes on to catch, which would make
    // every ordinary Ctrl+C look like a crash. Suppressed for exactly
    // that one payload type; anything else (a genuine bug) still hits
    // the normal hook, unchanged.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.payload().downcast_ref::<oderom_expr::Cancelled>().is_none() {
            default_hook(info);
        }
    }));

    let current_ctx: Arc<Mutex<Option<Arc<ExecutionContext>>>> = Arc::new(Mutex::new(None));
    {
        let current_ctx = current_ctx.clone();
        ctrlc::set_handler(move || {
            if let Some(ctx) = current_ctx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                ctx.cancel();
            }
            // Nothing running (idle at the prompt): rustyline is reading
            // the terminal in a mode where Ctrl+C arrives to *it*, not
            // as a process signal, so there is nothing to do here in
            // that case -- this handler existing at all is specifically
            // to survive the *other* case (a computation in flight,
            // where no readline call is blocking).
        })
        .expect("failed to install the Ctrl+C handler");
    }

    let mut repl = Repl::new(current_ctx);
    repl.run();
}

struct Repl {
    session: Session,
    editor: DefaultEditor,
    current_file: Option<PathBuf>,
    current_ctx: Arc<Mutex<Option<Arc<ExecutionContext>>>>,
    timeout: Duration,
}

impl Repl {
    fn new(current_ctx: Arc<Mutex<Option<Arc<ExecutionContext>>>>) -> Self {
        let mut editor = DefaultEditor::new().expect("failed to initialize the line editor");
        let _ = editor.load_history(&history_path()); // no history file yet on a fresh machine -- not an error
        Repl { session: Session::new(), editor, current_file: None, current_ctx, timeout: DEFAULT_TIMEOUT }
    }

    fn run(&mut self) {
        println!("ODEROM REPL. :load <file.od> to start, :quit or Ctrl+D to leave.");
        loop {
            match self.editor.readline("oderom> ") {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let _ = self.editor.add_history_entry(line);
                    if line == ":quit" {
                        break;
                    }
                    self.dispatch(line);
                }
                Err(ReadlineError::Interrupted) => {
                    // Ctrl+C at an empty/in-progress prompt line --
                    // standard readline behavior: cancel *that line*,
                    // reprompt. Does not touch `current_ctx`; nothing is
                    // running here by construction (readline only blocks
                    // between queries, never during one).
                    continue;
                }
                Err(ReadlineError::Eof) => break, // Ctrl+D
                Err(e) => {
                    eprintln!("readline error: {e}");
                    break;
                }
            }
        }
        let _ = self.editor.save_history(&history_path());
    }

    fn dispatch(&mut self, line: &str) {
        if let Some(rest) = line.strip_prefix(':') {
            let mut parts = rest.splitn(2, char::is_whitespace);
            let cmd = parts.next().unwrap_or("");
            let arg = parts.next().map(str::trim).unwrap_or("");
            match cmd {
                "load" => self.cmd_load(arg),
                "reload" => self.cmd_reload(),
                "defs" => self.cmd_defs(),
                "entries" => self.cmd_entries(),
                "recompute" => self.cmd_recompute(arg),
                "save" => self.cmd_save(arg),
                "timeout" => self.cmd_timeout(arg),
                _ => println!("unknown command :{cmd} -- :load :reload :defs :entries :recompute :save :timeout :quit"),
            }
        } else {
            self.cmd_query(line);
        }
    }

    fn cmd_load(&mut self, arg: &str) {
        if arg.is_empty() {
            println!("usage: :load <file.od>");
            return;
        }
        let path = PathBuf::from(arg);
        self.evaluate_file(&path);
        self.current_file = Some(path);
    }

    fn cmd_reload(&mut self) {
        let Some(path) = self.current_file.clone() else {
            println!("no file loaded yet -- :load <file.od> first");
            return;
        };
        self.evaluate_file(&path);
    }

    fn evaluate_file(&mut self, path: &PathBuf) {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                println!("could not read {}: {e}", path.display());
                return;
            }
        };
        match self.session.evaluate_definitions(source) {
            Ok(summary) => {
                println!("ok: {} definition(s), {}ms", summary.names.len(), summary.elapsed_ms);
                let stale = self.session.entries().iter().filter(|e| e.state.is_stale()).count();
                if stale > 0 {
                    println!("{stale} entr{} now OBSOLETE -- see :entries", if stale == 1 { "y" } else { "ies" });
                }
            }
            Err(CliError::Parse { message, position }) => match position {
                Some(p) => println!("parse error at line {}, column {}: {message}", p.line, p.column),
                None => println!("parse error: {message}"),
            },
            Err(e) => println!("error: {e}"),
        }
    }

    fn cmd_defs(&self) {
        let Some(document) = self.session.document() else {
            println!("no definitions evaluated yet");
            return;
        };
        let mut names: Vec<&String> = document.fingerprints.keys().collect();
        names.sort();
        if names.is_empty() {
            println!("(no named definitions)");
        }
        for name in names {
            println!("  {name}");
        }
    }

    fn cmd_entries(&self) {
        if self.session.entries().is_empty() {
            println!("(no entries yet)");
            return;
        }
        for (i, entry) in self.session.entries().iter().enumerate() {
            println!("{}", format_entry_line(i + 1, entry));
        }
    }

    fn cmd_recompute(&mut self, arg: &str) {
        if arg == "stale" {
            let stale_ids: Vec<EntryId> =
                self.session.entries().iter().filter(|e| e.state.is_stale()).map(|e| e.id).collect();
            if stale_ids.is_empty() {
                println!("nothing is obsolete");
                return;
            }
            let count = stale_ids.len();
            for id in stale_ids {
                self.recompute_one(id);
            }
            println!("recomputed {count} entr{}", if count == 1 { "y" } else { "ies" });
            return;
        }
        let Ok(index) = arg.parse::<usize>() else {
            println!("usage: :recompute <n> | :recompute stale");
            return;
        };
        let Some(id) = self.session.entries().get(index.wrapping_sub(1)).map(|e| e.id) else {
            println!("no entry #{index} -- see :entries");
            return;
        };
        self.recompute_one(id);
        if let Some(entry) = self.session.entries().iter().find(|e| e.id == id) {
            println!("{}", format_entry_line(index, entry));
        }
    }

    fn cmd_timeout(&mut self, arg: &str) {
        if arg.is_empty() {
            println!("current timeout: {:?}", self.timeout);
            return;
        }
        match arg.parse::<f64>() {
            Ok(secs) if secs > 0.0 && secs.is_finite() => {
                self.timeout = Duration::from_secs_f64(secs);
                println!("timeout set to {:?}", self.timeout);
            }
            _ => println!("usage: :timeout <seconds> (a positive number; e.g. :timeout 60) -- :timeout alone shows the current value"),
        }
    }

    /// Waits for the worker thread's result under `self.timeout` -- the
    /// wall-clock guardrail the CLI's own `--timeout`/`run_with_budget`
    /// has always had (`oderom-cli/src/commands.rs`) but the REPL never
    /// applied: `run_query` has no timeout of its own, and this used to
    /// be a plain, unbounded `rx.recv()`. On timeout, cancels `ctx` --
    /// the same flag Ctrl+C uses -- and keeps waiting: the worker thread
    /// still owns `self.session`'s state and must hand it back before
    /// this can return, but with cancellation now checked deep inside
    /// `normalize()`/`poly_gcd` (not only between components), that
    /// second wait is reliably short instead of open-ended.
    fn recv_with_timeout<T>(&self, rx: mpsc::Receiver<T>, ctx: &ExecutionContext) -> T {
        match rx.recv_timeout(self.timeout) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                println!("timed out after {:?} -- cancelling...", self.timeout);
                ctx.cancel();
                rx.recv().expect("worker thread never panics without sending first")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("worker thread disconnected without a result -- it must have panicked, which run_query is meant to catch internally (see oderom_expr::run_cancellable)")
            }
        }
    }

    fn cmd_save(&self, arg: &str) {
        if arg.is_empty() {
            println!("usage: :save <file>");
            return;
        }
        // Only `input`, never `state`/results (DESIGN-UI-SESSION.md
        // "Persistência": a saved output is exactly how a stale result
        // survives closing the app).
        let body: String = self.session.entries().iter().map(|e| format!("{}\n", e.input)).collect();
        match std::fs::write(arg, body) {
            Ok(()) => println!("saved {} entries to {arg}", self.session.entries().len()),
            Err(e) => println!("could not write {arg}: {e}"),
        }
    }

    fn cmd_query(&mut self, input: &str) {
        if self.session.document().is_none() {
            println!("no definitions evaluated yet -- :load <file.od> first");
            return;
        }

        let ctx = ExecutionContext::new();
        *self.current_ctx.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx.clone());

        // Progress (`ExecutionContext::set`) already prints its own
        // stage lines to stderr as the worker thread goes -- "appears
        // during the computation" is free. What this adds is the
        // one-line summary once it's done, and the thread boundary
        // Ctrl+C needs to interrupt something that isn't blocked on
        // readline.
        let (tx, rx) = mpsc::channel();
        let mut session = std::mem::take(&mut self.session);
        let input_owned = input.to_string();
        let worker_ctx = ctx.clone();
        let handle = std::thread::spawn(move || {
            let result = session.run_entry_with_context(input_owned, &worker_ctx);
            let _ = tx.send((session, result));
        });
        let (session, result) = self.recv_with_timeout(rx, &ctx);
        handle.join().ok();
        self.session = session;
        *self.current_ctx.lock().unwrap_or_else(|e| e.into_inner()) = None;

        match result {
            Ok(id) => {
                let index = self.session.entries().iter().position(|e| e.id == id).expect("just inserted") + 1;
                let entry = &self.session.entries()[index - 1];
                println!("{}", format_entry_line(index, entry));
                if let EntryState::Done { result, .. } = &entry.state {
                    println!("{}", result.unicode);
                }
            }
            Err(e) => println!("error: {e}"),
        }
    }

    fn recompute_one(&mut self, id: EntryId) {
        let ctx = ExecutionContext::new();
        *self.current_ctx.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx.clone());
        let mut session = std::mem::take(&mut self.session);
        let (tx, rx) = mpsc::channel();
        let worker_ctx = ctx.clone();
        let handle = std::thread::spawn(move || {
            let _ = session.recompute_entry_with_context(id, &worker_ctx);
            let _ = tx.send(session);
        });
        self.session = self.recv_with_timeout(rx, &ctx);
        handle.join().ok();
        *self.current_ctx.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// One line, unambiguous state marker -- explicit text
/// (`[OK]`/`[OBSOLETE]`/`[ERROR]`/`[PENDING]`), never just a subtle
/// formatting difference (DESIGN-UI-SESSION.md's own requirement,
/// carried over from the acceptance criterion: obsolescence has to be
/// impossible to miss).
fn format_entry_line(index: usize, entry: &Entry) -> String {
    match &entry.state {
        EntryState::Pending => format!("[{index}] {} -- [PENDING]", entry.input),
        EntryState::Running => format!("[{index}] {} -- [RUNNING]", entry.input),
        EntryState::Done { result, .. } => format!("[{index}] {} -- [OK] ({}ms)", entry.input, result.elapsed_ms),
        EntryState::Stale { result, .. } => {
            format!("[{index}] {} -- [OBSOLETE] (last computed {}ms; :recompute {index} to refresh)", entry.input, result.elapsed_ms)
        }
        // Etapa 3b (oderom-notebook's cancellation work) gave
        // cancellation its own `EntryState`, shared by this crate --
        // previously indistinguishable here from a genuine `[ERROR]`.
        EntryState::Cancelled => format!("[{index}] {} -- [CANCELLED]", entry.input),
        EntryState::Failed { message, line, column } => match (line, column) {
            (Some(l), Some(c)) => format!("[{index}] {} -- [ERROR] line {l}, column {c}: {message}", entry.input),
            _ => format!("[{index}] {} -- [ERROR] {message}", entry.input),
        },
    }
}
