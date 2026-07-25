//! ODEROM notebook shell -- Etapa 3a-2 (DESIGN-NOTEBOOK.md). All
//! geometry/algebra/rendering-format decisions happen in
//! `oderom-notebook`/`oderom-session`/`oderom-cli`; this crate only
//! translates that state into JSON DTOs the static frontend
//! (`dist/notebook.js`) can render, and translates button clicks back
//! into `oderom_notebook::Notebook` method calls. No parsing, no
//! `Target::Latex` line-splitting decision beyond "does this line
//! contain a backslash" (`dist/notebook.js`'s own doc comment explains
//! why that specific, narrow judgment call lives in JS and not Rust).

use oderom_notebook::{BeginExecution, Block, BlockId, BlockOutput, DeclarationStatus, EntryState, Notebook};
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

/// What the frontend actually needs to draw one block: never the raw
/// `oderom_notebook::Block`/`BlockOutput` types (those aren't `Serialize`,
/// deliberately -- their shape is this crate's own to define, not
/// `oderom-notebook`'s public API surface). Etapa 3a-2 doesn't visually
/// distinguish `confirmed`/`divergent` (that's Etapa 3b), but the DTO
/// already carries the real status string either way, so 3b is a
/// frontend-only change, not a new command.
#[derive(serde::Serialize)]
struct BlockDto {
    id: u64,
    source: String,
    output: OutputDto,
    /// Jupyter-style `In [n]` numbering -- `None` before this specific
    /// block has itself ever been the direct target of an execute (see
    /// `oderom_notebook::Block::execution_count`'s own doc comment for
    /// why it's per-block-explicitly-run, not per-reconstruction-swept-in).
    execution_count: Option<u64>,
    /// Etapa 3b (DESIGN-NOTEBOOK.md section 9): `true` when this
    /// block's displayed result no longer trustworthily reflects
    /// what's live -- purely a fact to render (amber marker, never red:
    /// this is not an error), never something this crate or the
    /// frontend decides or acts on. Always `false` when
    /// `execution_count` is `None`.
    obsolete: bool,
}

/// Everything one `list_blocks` call needs to hand the frontend to
/// redraw the whole notebook, including the header (current file name)
/// -- a single round trip rather than a second command the frontend
/// would have to remember to call in sync with the first.
#[derive(serde::Serialize)]
struct NotebookDto {
    blocks: Vec<BlockDto>,
    current_path: Option<String>,
}

/// One clickable, copyable piece of a result -- mirrors
/// `oderom_components::RenderedComponent` field for field. `latex` here
/// is deliberately the exact same clean "name = value" string a
/// component click should place on the clipboard: the frontend never
/// reassembles or re-derives what to copy from anything else on screen,
/// it just takes this field verbatim (`notebook.js`'s click handler).
/// `orbit_note`, when present, is shown subordinated (smaller, beside
/// or below `latex`) but is never part of what gets copied.
#[derive(serde::Serialize, Clone)]
struct ComponentDto {
    latex: String,
    orbit_note: Option<String>,
}

fn component_dto(c: &oderom_components::RenderedComponent) -> ComponentDto {
    ComponentDto { latex: c.formula.clone(), orbit_note: c.orbit_note.clone() }
}

#[derive(serde::Serialize)]
#[serde(tag = "kind")]
enum OutputDto {
    NeverRun,
    Declaration { status: String, message: Option<String> },
    Query {
        state: String,
        latex: Option<String>,
        /// Same result as `latex`, split per component -- see
        /// `ComponentDto`. Empty when `state` isn't `"done"`/`"stale"`
        /// (nothing to show yet), same as `latex` being `None` then.
        components: Vec<ComponentDto>,
        /// Plain-English lines belonging to the whole result, not any
        /// one component (truncation count, identically-zero count) --
        /// rendered as plain text, never through KaTeX, never
        /// copyable by a component click.
        summary: Vec<String>,
        message: Option<String>,
    },
    /// Etapa 3b (cancelamento, DESIGN-NOTEBOOK.md): the block's most
    /// recent execution attempt is running or ended in cancellation --
    /// `state` is `"running"` or `"cancelled"`. `previous`, if present,
    /// is an *earlier* settled result (never itself running/cancelled)
    /// to render alongside -- always obsolete when shown this way,
    /// unconditionally: any `previous` present here is, by
    /// construction, superseded by a newer attempt, whatever
    /// `BlockDto::obsolete` (a separate, position/self-edit-based
    /// signal) happens to say.
    Attempt { state: String, previous: Option<PreviousResultDto> },
    Unrecognized { message: String },
}

#[derive(serde::Serialize)]
struct PreviousResultDto {
    state: String,
    latex: Option<String>,
    components: Vec<ComponentDto>,
    summary: Vec<String>,
    message: Option<String>,
}

/// One `EntryState` as a DTO -- shared by `Query`'s own state and
/// `Attempt`'s `previous` so the two can never quietly disagree about
/// the same vocabulary (`"done"`/`"stale"`/`"failed"`/...) or about
/// which of `components`/`summary` gets populated together with
/// `latex`.
struct EntryDto {
    state: &'static str,
    latex: Option<String>,
    components: Vec<ComponentDto>,
    summary: Vec<String>,
    message: Option<String>,
}

fn entry_state_dto(state: &EntryState) -> EntryDto {
    let empty = || EntryDto { state: "", latex: None, components: Vec::new(), summary: Vec::new(), message: None };
    match state {
        EntryState::Pending => EntryDto { state: "pending", ..empty() },
        EntryState::Running => EntryDto { state: "running", ..empty() },
        EntryState::Done { result, .. } => EntryDto {
            state: "done",
            latex: Some(result.latex.clone()),
            components: result.components.iter().map(component_dto).collect(),
            summary: result.summary.clone(),
            ..empty()
        },
        EntryState::Stale { result, .. } => EntryDto {
            state: "stale",
            latex: Some(result.latex.clone()),
            components: result.components.iter().map(component_dto).collect(),
            summary: result.summary.clone(),
            ..empty()
        },
        EntryState::Cancelled => EntryDto { state: "cancelled", ..empty() },
        EntryState::Failed { message, .. } => EntryDto { state: "failed", message: Some(message.clone()), ..empty() },
    }
}

fn block_to_dto(block: &Block, notebook: &Notebook) -> BlockDto {
    let output = match &block.output {
        BlockOutput::NeverRun => OutputDto::NeverRun,
        BlockOutput::Declaration(status) => {
            let (status, message) = match status {
                DeclarationStatus::Confirmed => ("confirmed", None),
                DeclarationStatus::Divergent => ("divergent", None),
                DeclarationStatus::Error(msg) => ("error", Some(msg.clone())),
            };
            OutputDto::Declaration { status: status.to_string(), message }
        }
        BlockOutput::Query(entry_id) => match notebook.session().entries().iter().find(|e| e.id == *entry_id) {
            Some(entry) => {
                let e = entry_state_dto(&entry.state);
                OutputDto::Query { state: e.state.to_string(), latex: e.latex, components: e.components, summary: e.summary, message: e.message }
            }
            // Should not happen (a block only ever holds an EntryId
            // this same Session created), but a DTO layer reports an
            // honest "I don't know" rather than panicking on a display
            // path.
            None => OutputDto::Query {
                state: "missing".to_string(),
                latex: None,
                components: Vec::new(),
                summary: Vec::new(),
                message: Some("no matching session entry".to_string()),
            },
        },
        BlockOutput::Attempt { attempt, previous } => {
            let state = match notebook.session().entries().iter().find(|e| e.id == *attempt).map(|e| &e.state) {
                Some(EntryState::Cancelled) => "cancelled",
                // Running is overwhelmingly the expected case; any
                // other state here would mean `finish_query` landed
                // without the block's own output being updated to
                // match, which nothing in `oderom-notebook` does --
                // "running" is the honest default, not a guess dressed
                // up as one of the other named states.
                _ => "running",
            };
            let previous = previous.and_then(|prev_id| {
                notebook.session().entries().iter().find(|e| e.id == prev_id).map(|e| {
                    let e = entry_state_dto(&e.state);
                    PreviousResultDto { state: e.state.to_string(), latex: e.latex, components: e.components, summary: e.summary, message: e.message }
                })
            });
            OutputDto::Attempt { state: state.to_string(), previous }
        }
        BlockOutput::Unrecognized(message) => OutputDto::Unrecognized { message: message.clone() },
    };
    BlockDto { id: block.id.0, source: block.source.clone(), output, execution_count: block.execution_count, obsolete: block.is_obsolete() }
}

#[tauri::command]
fn list_blocks(state: State<AppState>) -> NotebookDto {
    let notebook = state.notebook.lock().unwrap();
    NotebookDto {
        blocks: notebook.blocks().iter().map(|b| block_to_dto(b, &notebook)).collect(),
        current_path: notebook.current_path().map(|p| p.display().to_string()),
    }
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

/// Etapa 3b, segunda parte (exclusão mútua, DESIGN-NOTEBOOK.md section
/// 10.8): what `execute_block` actually did, so the frontend can tell a
/// real start apart from a refusal instead of both looking like silent
/// success. `Blocked` carries the id of the block that is actually
/// running, so the status bar can name it (never a modal -- the user's
/// own requirement) rather than just saying "no").
#[derive(serde::Serialize)]
#[serde(tag = "kind")]
enum ExecuteOutcomeDto {
    Ok,
    Blocked { by: u64 },
    NotFound,
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

/// One entry for the "Galeria" picker -- the frontend never sees
/// [`oderom_notebook::gallery::GalleryEntry`] itself (that type is
/// `&'static str` fields meant for Rust callers, not `Serialize`), just
/// enough to fill a dropdown and show the two description lines
/// (`oderom-notebook/src/gallery.rs`'s own `title`/`description`/
/// `invariant` fields).
#[derive(serde::Serialize)]
struct GalleryEntryDto {
    name: String,
    title: String,
    description: String,
    invariant: String,
}

/// Every known gallery entry, in catalog order -- static data, so this
/// never needs `AppState` at all.
#[tauri::command]
fn gallery_list() -> Vec<GalleryEntryDto> {
    oderom_notebook::gallery::ENTRIES
        .iter()
        .map(|e| GalleryEntryDto { name: e.name.to_string(), title: e.title.to_string(), description: e.description.to_string(), invariant: e.invariant.to_string() })
        .collect()
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
