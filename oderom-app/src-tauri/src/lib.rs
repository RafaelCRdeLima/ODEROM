//! ODEROM notebook shell -- Etapa 3a-2 (DESIGN-NOTEBOOK.md). All
//! geometry/algebra/rendering-format decisions happen in
//! `oderom-notebook`/`oderom-session`/`oderom-cli`; this crate only
//! translates that state into the JSON DTOs the static frontend
//! (`dist/notebook.js`) renders, and translates button clicks back
//! into `oderom_notebook::Notebook` method calls. No parsing, no
//! `Target::Latex` line-splitting decision beyond "does this line
//! contain a backslash" (`dist/notebook.js`'s own doc comment explains
//! why that specific, narrow judgment call lives in JS and not Rust).
//!
//! The DTOs themselves live in `oderom-ui`, not here: `dist/` is served
//! to two backends -- this one and `oderom-wasm`, which runs the same
//! page in a browser -- and both must emit byte-identical JSON. One
//! definition, shared, is what makes that a compiler check instead of a
//! thing to remember. See `dist/LEIA-ME.md`.

use oderom_notebook::{BeginExecution, BlockId, Notebook};
// A *forma* de tudo o que atravessa a fronteira mora no `oderom-ui`, e
// nao aqui, porque o backend wasm (`oderom-wasm`) precisa produzir
// exatamente o mesmo JSON que estes comandos -- com uma definicao so, o
// compilador cobra o que antes dependia de lembrar. Ver o doc comment
// daquele crate e `dist/LEIA-ME.md`.
use oderom_ui::{ExecuteOutcomeDto, ExportOptionsDto, GalleryEntryDto, NotebookDto};
use std::sync::Mutex;
use tauri::{Manager, State};

/// Smoke-test command kept from Etapa 3a-1's toolchain proof -- no
/// `oderom-notebook` involved on purpose, so it stays useful as a
/// baseline check if a *later* frontend regression makes it unclear
/// whether the problem is Tauri/webkit itself or the notebook code atop
/// it.
#[tauri::command]
fn hello() -> String {
    "hello from Rust".to_string()
}

/// Headless-checkable proof the page loaded and ran its own JS, not
/// just that the window/process exist -- found necessary the hard way
/// (Etapa 3a-2's own session log: a webview that shows its own
/// "Connection failed" page is still a fully running process). Kept
/// permanently, called from `dist/index.html`'s `window.onload` --
/// every future frontend change gets this check for free.
#[tauri::command]
fn frontend_ready() {
    let path = std::env::temp_dir().join("oderom_app_frontend_ready");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let _ = std::fs::write(path, now.to_string());
}

/// `dist/keytest.js`'s only channel back to the test harness that
/// launched it (`tests/keymap.rs`) -- writes (not appends: one report
/// per run, never a growing log a stale run could leave behind to
/// confuse the next one) whatever JSON the test driver decides to
/// report. The path comes from `ODEROM_APP_TEST_REPORT_PATH`, set by
/// the Rust test that spawns this process, so parallel test runs (and
/// a real interactive session, which never sets it) never collide on
/// one shared file -- same reasoning as the REPL's `$HOME`-scoped
/// history file.
#[tauri::command]
fn ui_test_report(message: String) {
    let Ok(path) = std::env::var("ODEROM_APP_TEST_REPORT_PATH") else { return };
    let _ = std::fs::write(path, message);
}

struct AppState {
    notebook: Mutex<Notebook>,
    /// One long-lived clipboard handle, not a fresh `arboard::Clipboard`
    /// per command call -- X11 (unlike Windows/macOS) has no central
    /// clipboard store; the *owning process* serves paste requests for
    /// as long as it holds the selection, and arboard drops that
    /// ownership when its `Clipboard` value is dropped. A
    /// construct-set-drop-per-call version lost writes to a read that
    /// followed within milliseconds (found running this feature's own
    /// real-window test, `oderom-app/dist/keytest.js`'s Phase 8, which
    /// writes then immediately reads back to verify) -- arboard's own
    /// docs name this exact failure mode. Kept for the whole app's
    /// lifetime instead, same reasoning as `notebook` above.
    clipboard: Mutex<arboard::Clipboard>,
}

#[tauri::command]
fn list_blocks(state: State<AppState>) -> NotebookDto {
    oderom_ui::notebook_dto(&state.notebook.lock().unwrap())
}

#[tauri::command]
fn create_block(state: State<AppState>, after: Option<u64>, source: String) -> u64 {
    let mut notebook = state.notebook.lock().unwrap();
    notebook.create_block_after(after.map(BlockId), source).0
}

#[tauri::command]
fn edit_block(state: State<AppState>, id: u64, source: String) {
    state.notebook.lock().unwrap().edit_block(BlockId(id), source);
}


/// Etapa 3b (cancelamento, DESIGN-NOTEBOOK.md): returns as soon as a
/// query's computation has been handed off to its own thread -- never
/// holds `state.notebook`'s lock for the computation itself, which used
/// to be exactly what made one hung query (the non-reciprocal-metric
/// freeze this feature exists to fix) block every other command,
/// including this one and `cancel_block`, since they all need the same
/// lock. A declaration block (or a query that fails before there's
/// anything to thread off) still finishes here, synchronously, before
/// this command returns -- `Notebook::begin_execute`'s own doc comment
/// has the full split.
///
/// Etapa 3b, segunda parte: at most one execution runs at a time across
/// the whole notebook (`Notebook::begin_execute`'s own doc comment has
/// the full reasoning) -- a request that arrives while another block is
/// still running is refused outright, reported back as `Blocked`,
/// leaving the requested block completely untouched.
#[tauri::command]
fn execute_block(state: State<AppState>, app: tauri::AppHandle, id: u64) -> ExecuteOutcomeDto {
    let pending = {
        let mut notebook = state.notebook.lock().unwrap();
        match notebook.begin_execute(BlockId(id)) {
            BeginExecution::Started(pending) => Some(pending),
            BeginExecution::Done => None,
            BeginExecution::Blocked { by } => return ExecuteOutcomeDto::Blocked { by: by.0 },
            BeginExecution::NotFound => return ExecuteOutcomeDto::NotFound,
        }
    }; // lock released here, *before* any actual computation happens
    if let Some(pending) = pending {
        std::thread::spawn(move || {
            let result = pending.run();
            // Re-fetched via the `AppHandle` (not the borrowed `State`
            // this command started with, which does not live long
            // enough to reach a different thread) -- the same,
            // app-wide `AppState` either way.
            let state = app.state::<AppState>();
            state.notebook.lock().unwrap().finish_query(BlockId(id), result);
        });
    }
    ExecuteOutcomeDto::Ok
}

/// Requests cancellation of `id`'s in-flight execution, if any --
/// returns immediately either way (`Notebook::cancel_block`'s own doc
/// comment): this only asks the computation to stop at its next
/// checkpoint, it does not wait for it to actually do so. Reachable
/// promptly even while a long query is running precisely because
/// `execute_block` above no longer holds the notebook lock for that
/// duration -- the whole reason this split exists.
#[tauri::command]
fn cancel_block(state: State<AppState>, id: u64) {
    state.notebook.lock().unwrap().cancel_block(BlockId(id));
}

#[tauri::command]
fn delete_block(state: State<AppState>, id: u64) {
    state.notebook.lock().unwrap().delete_block(BlockId(id));
}

/// Places `text` on the OS clipboard -- backs the "click a result
/// component to copy its clean LaTeX" feature (`notebook.js`'s click
/// handler; what `text` actually *is* -- exactly `ComponentDto::latex`,
/// never the whole block's output, never the orbit note -- is decided
/// entirely in JS, this command just performs the OS-level write). Uses
/// `arboard` directly (a real OS clipboard, not `navigator.clipboard`)
/// so a click-driven copy is verifiable from a Rust test
/// (`read_clipboard_for_test` below) without depending on whichever
/// clipboard permission model the embedded webview happens to enforce.
#[tauri::command]
fn copy_to_clipboard(state: State<AppState>, text: String) -> Result<(), String> {
    state.clipboard.lock().unwrap().set_text(text).map_err(|e| e.to_string())
}

/// Test-only readback of the same OS clipboard `copy_to_clipboard`
/// writes to -- lets `tests/keymap.rs` assert on the *actual* clipboard
/// content after a synthetic click, not just that some click handler
/// ran. Harmless to leave reachable outside tests (it only ever reads,
/// same as any other application on the desktop already could), so
/// this is not gated behind `ODEROM_APP_TEST` the way `ui_test_report`
/// is -- that one writes to a path an attacker-controlled env var could
/// redirect, this one does not.
#[tauri::command]
fn read_clipboard_for_test(state: State<AppState>) -> Result<String, String> {
    state.clipboard.lock().unwrap().get_text().map_err(|e| e.to_string())
}


/// Every known gallery entry, in catalog order -- static data, so this
/// never needs `AppState` at all.
#[tauri::command]
fn gallery_list() -> Vec<GalleryEntryDto> {
    oderom_ui::gallery_entries()
}

/// What the "Exportar" picker can offer -- like `gallery_list`, static
/// data derived from the grammar itself, so no `AppState` either.
#[tauri::command]
fn export_options() -> ExportOptionsDto {
    oderom_ui::export_options()
}

/// `load NOME` (Rodada Galeria): pastes gallery entry `name`'s
/// declaration text as new blocks right after `after` (or at the end,
/// if `None`) -- [`oderom_notebook::Notebook::load_gallery_entry`]'s own
/// doc comment has the full reasoning (pure text injection, no
/// execution, existing blocks untouched). Returns the new blocks' ids
/// so the frontend can select/scroll to them, same shape `create_block`
/// already returns for one block.
#[tauri::command]
fn load_gallery(state: State<AppState>, after: Option<u64>, name: String) -> Result<Vec<u64>, String> {
    let mut notebook = state.notebook.lock().unwrap();
    notebook.load_gallery_entry(after.map(BlockId), &name).map(|ids| ids.into_iter().map(|id| id.0).collect()).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_notebook(state: State<AppState>, path: String) -> Result<(), String> {
    let mut notebook = state.notebook.lock().unwrap();
    oderom_notebook::save(&mut notebook, std::path::Path::new(&path)).map_err(|e| e.to_string())
}

/// "Limpar execução" (Botão 1): resets every block's execution state
/// (gutter back to `[ ]`, every displayed result gone) and discards all
/// session state (the compiled Model/aliases, every query entry, the
/// compute cache, the execution counter) -- block SOURCE TEXT and
/// `current_path` are the two things this never touches.
/// `Notebook::clear_execution`'s own doc comment has the full contract
/// and the reasoning for why it's provably equivalent to reopening this
/// exact text as a new file. Reversible (just re-execute), so unlike
/// [`new_notebook`] this needs no confirmation -- nothing the user wrote
/// is ever at risk. Never executes anything itself, never touches disk.
#[tauri::command]
fn clear_execution(state: State<AppState>) {
    state.notebook.lock().unwrap().clear_execution();
}

/// A genuinely blank notebook -- one empty block (the same dashed
/// "trailing empty" placeholder a notebook with a trailing blank block
/// already shows, `notebook.js`'s own `isTrailingEmpty`), never
/// [`seed_example`]'s worked demonstration: "blank" means blank, with
/// somewhere to start typing.
fn blank_notebook() -> Notebook {
    let mut notebook = Notebook::new();
    notebook.create_block_after(None, String::new());
    notebook
}

/// "Novo caderno" (Botão 2): discards every block -- text included --
/// and starts over blank, with the same fresh session state
/// `clear_execution` above also produces (there is no longer anything
/// left to carry session state about). Destructive (unlike
/// `clear_execution`, un-executed source text is genuinely gone, not
/// recoverable by re-running anything) -- the frontend is the one that
/// gates this behind a confirmation before ever invoking it; this
/// command itself performs the replacement unconditionally the moment
/// it's called, the same "Rust decides state, JS decides when to ask"
/// split every other command here already follows. Never touches
/// `current_path`/disk in the sense of a file on disk being deleted or
/// modified -- if the notebook on screen came from a file, that file is
/// completely untouched; only what's displayed is replaced (this is
/// `AppState.notebook`'s in-memory value, the exact same thing
/// `open_notebook` already replaces wholesale).
#[tauri::command]
fn new_notebook(state: State<AppState>) {
    *state.notebook.lock().unwrap() = blank_notebook();
}

#[tauri::command]
fn open_notebook(state: State<AppState>, path: String) -> Result<(), String> {
    let loaded = oderom_notebook::load(std::path::Path::new(&path)).map_err(|e| e.to_string())?;
    *state.notebook.lock().unwrap() = loaded;
    Ok(())
}

/// Reissner-Nordstrom -- this project's own standing acceptance fixture
/// (`oderom-components/tests/reissner_nordstrom.rs`), not a made-up
/// example. A fresh notebook opens with this loaded, never blank
/// (DESIGN-NOTEBOOK.md's Etapa 3a-2 scope), but nothing is executed on
/// startup -- "nada recalcula sozinho" applies here exactly as it does
/// to opening a saved notebook (`oderom_notebook::load`'s own doc
/// comment): the blocks show their source; Shift+Enter is still the
/// only thing that ever runs anything.
fn seed_example() -> Notebook {
    let mut notebook = Notebook::new();
    let a = notebook.create_block_after(None, "manifold M dim 4\nbundle TM on M dim 4".to_string());
    let b = notebook.create_block_after(Some(a), "chart schw on M coords (t, r, theta, phi)".to_string());
    notebook.create_block_after(
        Some(b),
        "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}"
            .to_string(),
    );
    notebook.create_block_after(None, "kretschmann".to_string());
    notebook
}

/// `ODEROM_APP_TEST` (any value, presence is all that's checked) picks
/// `dist/keytest.html` instead of the real `dist/index.html` --
/// `tests/keymap.rs` sets it when spawning this binary for automated
/// UI testing, so that test drives the *actual* `notebook.js`/
/// `oderom-mode.js`/CodeMirror/KaTeX files (never a reimplementation:
/// `keytest.html` includes the exact same `<script>` tags `index.html`
/// does), just with a small driver script appended that fires
/// synthetic-but-realistic key/mouse events instead of a person.
/// `tauri.conf.json`'s own `app.windows` is deliberately empty so this
/// is the only place a window gets created either way -- one decision
/// point, not two that could disagree.
fn frontend_entry_point() -> &'static str {
    if std::env::var("ODEROM_APP_TEST").is_ok() {
        "keytest.html"
    } else {
        "index.html"
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        hello,
        frontend_ready,
        ui_test_report,
        list_blocks,
        create_block,
        edit_block,
        execute_block,
        cancel_block,
        delete_block,
        save_notebook,
        open_notebook,
        clear_execution,
        new_notebook,
        gallery_list,
        export_options,
        load_gallery,
        copy_to_clipboard,
        read_clipboard_for_test,
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      let clipboard = arboard::Clipboard::new().expect("no clipboard available -- this app needs a real X11/Wayland display");
      app.manage(AppState { notebook: Mutex::new(seed_example()), clipboard: Mutex::new(clipboard) });
      tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(frontend_entry_point().into()))
        .title("ODEROM")
        .inner_size(1000.0, 720.0)
        .build()?;
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
