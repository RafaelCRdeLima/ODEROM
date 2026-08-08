// ODEROM notebook frontend -- Etapa 3a-2 (DESIGN-NOTEBOOK.md). This
// file renders state and forwards user actions; it never decides
// anything about geometry, algebra, or which components are shown --
// every one of those decisions already happened in Rust
// (oderom-notebook/oderom-session/oderom-cli) by the time a block's DTO
// reaches here (src-tauri/src/lib.rs's BlockDto), including which text
// is a formula and which is a plain-English caption
// (`RenderedComponent`'s own split, see `renderComponents`'s doc
// comment) -- this file draws that pre-made split, it does not make it.

// Fornecido por backend.js, que decide entre Tauri e wasm. Ver
// LEIA-ME.md neste diretório antes de editar este arquivo.
const invoke = window.ODEROM_invoke;

// block id -> CodeMirror instance, so Shift+Enter/edits can read the
// live editor content without a round trip through the DOM.
let editors = {};

// Etapa 3b (cancelamento): the highest `execution_count` actually
// applied to the DOM for each block id -- kept up to date by, and
// guards, `updateBlockChrome` only (see that function's own comment).
// `refresh()` deliberately does *not* consult this: it always shows
// exactly what its own `list_blocks()` call just returned, full stop --
// see that function's own comment on why the one real race this caused
// was fixed by removing an unnecessary caller, not by second-guessing
// a fresh response here too (tried first; made staleness bugs *more*
// likely elsewhere, not less, by second-guessing responses that were
// never stale to begin with).
let lastAppliedExecutionCount = {};

// Resolves once the post-refresh layout pass (and, if requested, the
// real .focus() call) has actually happened -- callers that need to
// chain further actions after a specific block is focused (none do
// today, but returning a promise costs nothing and avoids a future
// caller having to guess whether focus already landed).
function refresh(focusId) {
  return invoke("list_blocks").then(
    ({ blocks, current_path }) =>
      new Promise((resolve) => {
        renderHeader(current_path);
        renderStatusBar(blocks);

        const container = document.getElementById("blocks");
        container.innerHTML = "";
        editors = {};
        blocks.forEach((block, i) => {
          const isTrailingEmpty = i === blocks.length - 1 && block.source === "" && block.output.kind === "NeverRun";
          container.appendChild(renderBlock(block, isTrailingEmpty));
        });
        // CodeMirror 5's own internal size/content measurements are
        // wrong if taken before the browser has actually laid out the
        // DOM node it was just constructed in -- a well-known CM5
        // gotcha, and exactly what a full rebuild-from-scratch on every
        // execute (see `container.innerHTML = ""` above) runs into on
        // *every* refresh, not just the first one.
        // `requestAnimationFrame` waits for that layout pass before
        // asking every instance to remeasure itself -- and, if
        // `focusId` was given, before calling the REAL `.focus()` on
        // that block's (freshly reconstructed) CodeMirror instance.
        // Focusing before this layout pass would call `.focus()` on an
        // editor CodeMirror itself hasn't finished measuring yet.
        requestAnimationFrame(() => {
          for (const id in editors) {
            editors[id].refresh();
          }
          if (focusId !== undefined && editors[focusId]) {
            editors[focusId].focus();
          }
          resolve();
        });
      }),
  );
}

function renderHeader(currentPath) {
  const nameEl = document.getElementById("doc-name");
  const pathInput = document.getElementById("path-input");
  if (currentPath) {
    const base = currentPath.split(/[\\/]/).pop();
    nameEl.textContent = base;
    // Only prefill when the user hasn't typed something of their own --
    // never clobber an in-progress "Abrir"/"Salvar" path they're
    // editing.
    if (document.activeElement !== pathInput) {
      pathInput.value = currentPath;
    }
  } else {
    nameEl.textContent = "sem título";
  }
}

// Etapa 3b, segunda parte (exclusão mútua, DESIGN-NOTEBOOK.md section
// 10.8): the *content* here is always derived live from `blocks` -- the
// same per-block DTO `list_blocks` already returns -- so it never goes
// stale, recomputed on every poll (`pollUntilSettled`) and every full
// `refresh()`. This alone is NOT what makes a refusal visible, though:
// this message was already on screen, unchanged, before a refused key
// was pressed (that is the entire reason the key was refused), so by
// itself it has no perceptible edge at the moment of the keypress --
// indistinguishable, from the user's seat, from the key doing nothing
// at all. `flashRefusal` (below) is what actually answers that: a
// separate, momentary, retriggerable event layered on top of this
// always-correct ambient text, never a replacement for it.
function renderStatusBar(blocks) {
  const left = document.getElementById("status-left");
  const running = blocks.find((b) => b.output.kind === "Attempt" && b.output.state === "running");
  if (running) {
    left.textContent = `bloco [${running.execution_count}] em execução · outros blocos ficam bloqueados até terminar ou cancelar`;
    return;
  }
  const executed = blocks.filter((b) => b.execution_count !== null && b.execution_count !== undefined).length;
  left.textContent = `pronto · ${blocks.length} bloco${blocks.length === 1 ? "" : "s"} · ${executed} executado${executed === 1 ? "" : "s"}`;
}

// The lightweight counterpart to `refresh()` for the one case where a
// full rebuild would be actively wrong: `execute_block` came back
// `Blocked`. Only re-fetches and repaints the status bar -- never
// touches `editors`/the block DOM, so it cannot steal focus from
// whatever the user is doing (a bare `refresh()` with no `focusId`
// leaves nothing with real DOM focus at all, a real bug found and
// fixed earlier in this same feature; the blocked path must not
// reintroduce it). Satisfies criterion (e): blocking execution must
// never block editing/focus elsewhere in the window.
async function refreshStatusBarOnly() {
  const { blocks } = await invoke("list_blocks");
  renderStatusBar(blocks);
}

// Etapa 3b, segunda parte: `renderStatusBar`'s busy message is
// ambient -- unchanged by a refusal that happens while it was already
// showing, which by itself is indistinguishable from the key having
// done nothing at all (see notebook.css's own comment on
// `.refusal-flash`). This is the actual, momentary event: flashes the
// status bar AND the specific block (`byBlockId`, the same id
// `execute_block`'s `Blocked{by}` already returns) that is occupying
// execution -- a real transition at the instant of the refused key,
// never a substitute for the ambient message, only its missing edge.
//
// `refusalPulseSeq` is a monotonically increasing counter, stamped
// into `dataset.refusalPulse` on both elements -- the reliable signal
// anything (a test, or a future feature) can compare against, since
// the CSS animation's exact fade duration is cosmetic and nothing
// should depend on its timing.
let refusalPulseSeq = 0;

function flashRefusal(byBlockId) {
  refusalPulseSeq += 1;
  const seq = String(refusalPulseSeq);

  const statusEl = document.getElementById("status-left");
  statusEl.dataset.refusalPulse = seq;
  restartFlash(statusEl, "refusal-flash");

  const cm = editors[byBlockId];
  const wrapper = cm && cm.getWrapperElement().closest(".block");
  if (wrapper) {
    wrapper.dataset.refusalPulse = seq;
    restartFlash(wrapper, "refused-flash");
  }
}

// Removes then re-adds `className`, forcing a reflow in between --
// re-adding a class that is already present does not restart a CSS
// animation on its own, so without the forced reflow a refusal that
// lands while an earlier flash is still fading would be silently
// absorbed instead of restarting the halo.
function restartFlash(el, className) {
  el.classList.remove(className);
  void el.offsetWidth;
  el.classList.add(className);
  setTimeout(() => el.classList.remove(className), 700);
}

// The block's own delete button, or (while its most recent execution
// attempt is still running) a cancel button in the same slot -- never
// both. Shared by `renderBlock` (full render) and `updateBlockChrome`
// (in-place update once a background execution starts/settles), so the
// two can never quietly build it differently.
function buildActionButton(blockId, isRunning) {
  if (isRunning) {
    const btn = document.createElement("button");
    btn.className = "block-cancel";
    btn.title = "Cancelar execução";
    btn.textContent = "Cancelar";
    btn.onclick = async () => {
      // No `refresh()`/DOM update here -- cancelling only *requests*
      // it (`Notebook::cancel_block`'s own doc comment); the block
      // stays "running"-styled until `pollUntilSettled` (already
      // watching it since the execution began) observes the real
      // transition to "cancelled" and updates it then. Racing ahead of
      // that would show "cancelled" before the worker thread has
      // actually stopped.
      await invoke("cancel_block", { id: blockId });
    };
    return btn;
  }
  const btn = document.createElement("button");
  btn.className = "block-delete";
  btn.title = "Apagar bloco";
  btn.setAttribute("aria-label", "Apagar bloco");
  btn.textContent = "×";
  btn.onclick = async () => {
    await invoke("delete_block", { id: blockId });
    await refresh();
  };
  return btn;
}

function renderBlock(block, isTrailingEmpty) {
  const wrapper = document.createElement("div");
  const isRunning = block.output.kind === "Attempt" && block.output.state === "running";
  const isCancelled = block.output.kind === "Attempt" && block.output.state === "cancelled";
  wrapper.className =
    "block" +
    (isTrailingEmpty ? " trailing-empty" : "") +
    // The generic obsolete stripe/marker is suppressed while running
    // or cancelled -- those states already have their own complete
    // visual treatment (notebook.css's own comment on `.block.running`/
    // `.block.cancelled`); layering the generic one on top would put
    // two competing stripes on the same block for no added information
    // ("resultado obsoleto" already appears, unconditionally, next to
    // `previous`'s own content below, regardless of this class).
    (block.obsolete && !isRunning && !isCancelled ? " obsolete" : "") +
    (isRunning ? " running" : "") +
    (isCancelled ? " cancelled" : "");

  const gutter = document.createElement("div");
  gutter.className = "block-gutter";
  gutter.textContent = block.execution_count != null ? `[${block.execution_count}]` : "[ ]";
  wrapper.appendChild(gutter);

  const main = document.createElement("div");
  main.className = "block-main";
  wrapper.appendChild(main);

  wrapper.appendChild(buildActionButton(block.id, isRunning));

  const editorHost = document.createElement("div");
  editorHost.className = "block-editor";
  main.appendChild(editorHost);

  let placeholderEl = null;
  if (isTrailingEmpty) {
    placeholderEl = document.createElement("div");
    placeholderEl.className = "block-placeholder";
    placeholderEl.textContent = "Shift+Enter para executar";
    main.appendChild(placeholderEl);
  }

  // Jupyter-style execute keys, not Mathematica's -- registered via
  // `extraKeys` on the instance itself (never a document/container
  // listener: CodeMirror consumes keys inside it and they do not
  // bubble, so a listener anywhere else silently never sees them --
  // the exact bug an earlier round of this session shipped and had to
  // diagnose). Each handler flushes the current text immediately first
  // (never waits for the debounced sync below) -- executing must always
  // see exactly what's on screen right now, per DESIGN-NOTEBOOK.md
  // section 2 ("o texto ATUAL de todos os blocos de declaração").
  //
  // "Nada recalcula sozinho" still governs every one of these: moving
  // focus to another block never executes it; the only thing that runs
  // a block is a key that names it explicitly, right here.
  const cm = CodeMirror(editorHost, {
    value: block.source,
    mode: "oderom",
    lineNumbers: false,
    viewportMargin: Infinity,
    extraKeys: {
      // Execute, then move focus to the next block -- or, if this is
      // the last block, create a new empty one and focus that instead.
      // The new/next block is only ever focused, never executed.
      //
      // `execute_block` (Etapa 3b, cancelamento) returns as soon as a
      // query's computation has been handed off to its own thread, not
      // once it finishes -- focus already moves on right after, and
      // `pollUntilSettled` (fire-and-forget, never awaited) is what
      // updates *this* block in place once it actually settles,
      // independently of wherever the user has since moved on to.
      "Shift-Enter": async (instance) => {
        await invoke("edit_block", { id: block.id, source: instance.getValue() });
        // Etapa 3b, segunda parte (exclusão mútua): `execute_block` now
        // reports whether it actually started -- while another block is
        // running, this is refused outright (`Blocked`), and this block
        // is left completely untouched (no execution_count bump, no
        // output change, nothing to poll for). Only the status bar
        // (already showing which block is busy) gets refreshed; no
        // focus move, no new block, and deliberately no full `refresh()`
        // -- see `refreshStatusBarOnly`'s own comment for why.
        const outcome = await invoke("execute_block", { id: block.id });
        if (outcome.kind !== "Ok") {
          if (outcome.kind === "Blocked") {
            flashRefusal(outcome.by);
            await refreshStatusBarOnly();
          }
          return;
        }
        pollUntilSettled(block.id);
        const { blocks } = await invoke("list_blocks");
        const idx = blocks.findIndex((b) => b.id === block.id);
        const isLast = idx === blocks.length - 1;
        const targetId = isLast ? await invoke("create_block", { after: block.id, source: "" }) : blocks[idx + 1].id;
        await refresh(targetId);
      },
      // Execute, keep focus on the same block. Creates nothing.
      "Ctrl-Enter": async (instance) => {
        await invoke("edit_block", { id: block.id, source: instance.getValue() });
        const outcome = await invoke("execute_block", { id: block.id });
        if (outcome.kind !== "Ok") {
          if (outcome.kind === "Blocked") {
            flashRefusal(outcome.by);
            await refreshStatusBarOnly();
          }
          return;
        }
        // No `refresh()` here on purpose -- focus never actually leaves
        // this block for Ctrl-Enter (its own CodeMirror instance is
        // never torn down), and `pollUntilSettled` below reflects the
        // resulting state -- starting from "running" -- on its own
        // first iteration. An explicit `refresh(block.id)` here used to
        // race with it: two independent `list_blocks()` round trips for
        // the same block have no ordering guarantee, so this call's own
        // (possibly slightly stale-by-the-time-it-resolves) response
        // could land *after* `pollUntilSettled`'s fresher one already
        // had, visibly regressing a block that had already settled
        // (e.g. to "cancelled") back to "running" -- found by direct
        // evidence during this feature's own testing. Removing the
        // redundant call removes the race outright, rather than trying
        // to out-guess which of two responses for the same block is
        // the fresher one.
        pollUntilSettled(block.id);
      },
      // Execute, then insert a new empty block immediately below --
      // even if one already follows -- and focus the new one.
      "Alt-Enter": async (instance) => {
        await invoke("edit_block", { id: block.id, source: instance.getValue() });
        const outcome = await invoke("execute_block", { id: block.id });
        if (outcome.kind !== "Ok") {
          if (outcome.kind === "Blocked") {
            flashRefusal(outcome.by);
            await refreshStatusBarOnly();
          }
          return;
        }
        pollUntilSettled(block.id);
        const newId = await invoke("create_block", { after: block.id, source: "" });
        await refresh(newId);
      },
    },
  });
  editors[block.id] = cm;

  // Focus ring on the whole block (gutter + editor), never a filled
  // background -- CodeMirror toggles `.CodeMirror-focused` on its own
  // wrapper already; this just mirrors that onto the block as a whole
  // so the ring can sit outside the gutter too.
  cm.on("focus", () => wrapper.classList.add("focused"));
  cm.on("blur", () => wrapper.classList.remove("focused"));

  if (placeholderEl) {
    const syncPlaceholder = () => {
      placeholderEl.style.display = cm.getValue() === "" ? "block" : "none";
    };
    cm.on("change", syncPlaceholder);
    cm.on("focus", syncPlaceholder);
  }

  // "editar -> divergente" is meant to be immediate (DESIGN-NOTEBOOK.md
  // section 3), not deferred until the editor loses focus -- debounced,
  // not per-keystroke, so typing doesn't flood IPC. This never triggers
  // a `refresh()` on its own: rebuilding every editor mid-keystroke
  // would drop the user's cursor position, which nothing here is worth
  // doing except right after an action that genuinely changed the
  // block list or a block's own displayed output (execute/create/
  // delete/save/open).
  let syncTimer = null;
  cm.on("change", (instance) => {
    if (syncTimer) clearTimeout(syncTimer);
    syncTimer = setTimeout(() => {
      invoke("edit_block", { id: block.id, source: instance.getValue() });
    }, 400);
  });

  const output = document.createElement("div");
  output.className = "block-output";
  renderOutput(output, block.output, block.obsolete);
  main.appendChild(output);

  return wrapper;
}

// Etapa 3b (cancelamento): watches one block that just started
// executing until it settles (normally, cancelled, or deleted),
// updating it *in place* (`updateBlockChrome`) on every poll --
// deliberately never calls the full `refresh()`, and deliberately
// fire-and-forget (every caller starts this without awaiting it): the
// entire point of moving execution off the command thread was to keep
// the rest of the window usable while this runs, and awaiting it here
// would put that right back.
//
// The whole loop body is one try/catch, on purpose: this function is
// fire-and-forget, so nothing else is watching it, which means an
// uncaught exception here does not fail loudly -- it just kills this
// one loop's own `for` silently, mid-poll, leaving whatever the DOM
// last showed (very possibly "running") permanently stuck, since
// nothing else will ever call `updateBlockChrome` for this block
// again. Found the hard way: a real, if rare, exception on some poll
// iteration (the exact trigger not pinned down -- concurrent
// `invoke()` load from several blocks' own poll loops all running at
// once is the leading suspect) left a genuinely-cancelled block
// showing "running" forever during this feature's own UI testing.
// Retrying instead of dying matches `waitFor`'s own established
// pattern: treat an exception as "not ready yet", not as fatal.
async function pollUntilSettled(blockId) {
  for (;;) {
    try {
      const { blocks } = await invoke("list_blocks");
      // Free, since `blocks` was already fetched for `updateBlockChrome`
      // below -- see `renderStatusBar`'s own comment for why this is
      // enough to keep the busy-state message live without a dedicated
      // "refusal" event.
      renderStatusBar(blocks);
      const dto = blocks.find((b) => b.id === blockId);
      if (!dto) return; // deleted meanwhile -- nothing left to update
      updateBlockChrome(blockId, dto);
      const stillRunning = dto.output.kind === "Attempt" && dto.output.state === "running";
      if (!stillRunning) return;
    } catch (e) {
      // Fall through to the retry below.
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
}

// Updates one block's gutter/classes/action-button/output *without*
// touching its CodeMirror instance or any other block -- unlike
// `refresh()`, which tears down and rebuilds every editor on the page.
// A full refresh() here would be actively wrong, not just wasteful: by
// the time a background query settles, the user has very likely
// already moved on to editing or focusing a *different* block (the
// whole reason execution stopped blocking anything in the first
// place), and rebuilding every instance would drop whatever cursor
// position/focus they have there.
//
// `lastAppliedExecutionCount` (declared up top, alongside `editors`)
// gets updated and consulted here only -- see its own comment for why
// a plain `dto.execution_count < seen` check is needed at all: two
// `invoke("list_blocks")` round trips for the same block have no
// ordering guarantee relative to each other.
function updateBlockChrome(blockId, dto) {
  const seen = lastAppliedExecutionCount[blockId];
  if (dto.execution_count != null && seen != null && dto.execution_count < seen) {
    return; // a stale response for an older attempt -- a newer one already rendered
  }
  if (dto.execution_count != null) {
    lastAppliedExecutionCount[blockId] = dto.execution_count;
  }

  const cm = editors[blockId];
  if (!cm) return; // deleted, or a full refresh() already rebuilt everything (and so already reflects current state)
  const wrapper = cm.getWrapperElement().closest(".block");
  if (!wrapper) return;

  const isRunning = dto.output.kind === "Attempt" && dto.output.state === "running";
  const isCancelled = dto.output.kind === "Attempt" && dto.output.state === "cancelled";
  wrapper.classList.toggle("obsolete", !!dto.obsolete && !isRunning && !isCancelled);
  wrapper.classList.toggle("running", isRunning);
  wrapper.classList.toggle("cancelled", isCancelled);

  const gutter = wrapper.querySelector(".block-gutter");
  if (gutter) {
    gutter.textContent = dto.execution_count != null ? `[${dto.execution_count}]` : "[ ]";
  }

  const existingBtn = wrapper.querySelector(".block-delete, .block-cancel");
  const hasCancelBtn = !!existingBtn && existingBtn.classList.contains("block-cancel");
  if (isRunning !== hasCancelBtn) {
    if (existingBtn) existingBtn.remove();
    wrapper.appendChild(buildActionButton(blockId, isRunning));
  }

  const output = wrapper.querySelector(".block-output");
  if (output) {
    renderOutput(output, dto.output, dto.obsolete);
  }
}

// `obsolete` (Etapa 3b, DESIGN-NOTEBOOK.md section 9) never changes
// WHAT is rendered here -- the exact same result the block already had
// -- only whether the "resultado obsoleto" label is prepended above it.
// The gutter's own amber marker and left stripe (notebook.css) are the
// other two signals; this is the third, purely textual one, added only
// where there's an actual result on screen to label.
function renderOutput(el, output, obsolete) {
  el.innerHTML = "";
  if (output.kind === "NeverRun") {
    return;
  }
  if (output.kind === "Attempt") {
    // Etapa 3b (cancelamento): running or cancelled -- `previous`, if
    // present, is rendered unconditionally as obsolete (never gated on
    // the generic `obsolete` flag below, which describes Query/
    // Declaration blocks' own position/self-edit history, not this):
    // any `previous` here is, by construction, superseded by a newer
    // attempt.
    const label = document.createElement("div");
    label.className = output.state === "running" ? "running-label" : "cancelled-label";
    label.textContent = output.state === "running" ? "calculando..." : "execução cancelada";
    el.appendChild(label);
    if (output.previous) {
      const obsLabel = document.createElement("div");
      obsLabel.className = "obsolete-label";
      obsLabel.textContent = "resultado obsoleto";
      el.appendChild(obsLabel);
      renderComponents(el, output.previous.components, output.previous.summary);
      if (output.previous.message) {
        appendErrorText(el, output.previous.message);
      }
    }
    return;
  }
  if (obsolete) {
    const label = document.createElement("div");
    label.className = "obsolete-label";
    label.textContent = "resultado obsoleto";
    el.appendChild(label);
  }
  if (output.kind === "Declaration") {
    // Confirmed/Divergent still render no body content -- a
    // declaration block's only content-shaped output is an error, if
    // any. The gutter (amber marker + stripe) and, when there's a
    // result, the label above are what carry obsolescence for this
    // kind.
    if (output.message) {
      appendErrorText(el, output.message);
    }
    return;
  }
  if (output.kind === "Query") {
    renderComponents(el, output.components, output.summary);
    if (output.message) {
      appendErrorText(el, output.message);
    }
    return;
  }
  if (output.kind === "Unrecognized") {
    appendErrorText(el, output.message);
  }
}

function appendErrorText(el, message) {
  const p = document.createElement("div");
  p.className = "error-text";
  p.textContent = message;
  el.appendChild(p);
}

// Renders a result's already-structured pieces
// (`oderom_components::RenderedComponent` by way of `ComponentDto` --
// see src-tauri/src/lib.rs) -- never a single blob to split by
// sniffing for a backslash the way this used to work. Rust already
// decided, per component, what is real math (`comp.latex`, fed to
// KaTeX as its own call and nothing else) and what is a plain-English
// annotation (`comp.orbit_note`, if any -- rendered as ordinary text
// right beside it, never inside the same KaTeX call, which is exactly
// what used to come out italic and unspaced: "4componentsbysymmetry").
// `summary` lines (truncation count, identically-zero count) belong to
// no single component and are always plain text.
//
// This is the only judgment call left in JS for this feature, and it
// is not a math decision: which text is a formula and which is a
// caption was already decided in Rust (`RenderedComponent`'s own split)
// before this DTO ever reached the frontend -- this function just
// draws the two differently and wires up the click-to-copy affordance.
function renderComponents(el, components, summary) {
  for (const comp of components || []) {
    const row = document.createElement("div");
    row.className = "component-row";
    row.title = "clique para copiar o LaTeX";

    const formula = document.createElement("div");
    formula.className = "latex-line";
    try {
      katex.render(comp.latex, formula, { throwOnError: false });
    } catch (e) {
      formula.textContent = comp.latex;
    }
    row.appendChild(formula);

    if (comp.orbit_note) {
      const note = document.createElement("span");
      note.className = "component-note";
      note.textContent = `(${comp.orbit_note})`;
      row.appendChild(note);
    }

    row.addEventListener("click", () => copyComponentLatex(row, comp.latex));
    el.appendChild(row);
  }
  for (const line of summary || []) {
    const lineEl = document.createElement("div");
    lineEl.className = "result-summary-line";
    lineEl.textContent = line;
    el.appendChild(lineEl);
  }
}

// Copies exactly `latex` (one component's clean formula, never the
// orbit note, never any other component, never the whole block) to the
// real OS clipboard via a Rust command (`copy_to_clipboard`) rather
// than `navigator.clipboard` directly -- keeps this verifiable from a
// real Rust test (`read_clipboard_for_test`) regardless of whichever
// clipboard permission model the embedded webview enforces. Feedback
// is a small "copiado" badge that appears then removes itself -- never
// a modal (explicit requirement), and never left hanging if a second
// click lands before the first badge's timeout fires.
async function copyComponentLatex(row, latex) {
  try {
    await invoke("copy_to_clipboard", { text: latex });
  } catch (e) {
    return;
  }
  const existing = row.querySelector(".copied-flash");
  if (existing) existing.remove();
  const flash = document.createElement("span");
  flash.className = "copied-flash";
  flash.textContent = "copiado";
  row.appendChild(flash);
  setTimeout(() => flash.remove(), 1200);
}

document.getElementById("save-btn").addEventListener("click", async () => {
  const path = document.getElementById("path-input").value;
  if (!path) return;
  try {
    await invoke("save_notebook", { path });
    await refresh();
  } catch (e) {
    window.alert("Erro ao salvar: " + e);
  }
});

document.getElementById("open-btn").addEventListener("click", async () => {
  const path = document.getElementById("path-input").value;
  if (!path) return;
  try {
    await invoke("open_notebook", { path });
    await refresh();
  } catch (e) {
    window.alert("Erro ao abrir: " + e);
  }
});

// "Limpar execução" (Botão 1): reversible (re-executing brings any
// result right back), so this fires immediately on click -- no
// confirmation, matching its own safety, unlike "Novo caderno" below.
// All the actual state work happens in Rust (`clear_execution`); this
// only asks for it, then redraws whatever came back, the same
// "Rust decides state, JS redraws" split every other button here uses.
document.getElementById("clear-execution-btn").addEventListener("click", async () => {
  await invoke("clear_execution");
  await refresh();
});

// "Novo caderno" (Botão 2): destructive -- unlike every other button in
// this header, this one does NOT act on click. It only opens the
// confirmation panel below; `new_notebook` is invoked from exactly one
// place, the panel's own "Apagar tudo" button.
document.getElementById("new-notebook-btn").addEventListener("click", () => {
  document.getElementById("new-notebook-confirm-panel").hidden = false;
});

function closeNewNotebookConfirmPanel() {
  document.getElementById("new-notebook-confirm-panel").hidden = true;
}

document.getElementById("new-notebook-cancel-btn").addEventListener("click", closeNewNotebookConfirmPanel);

document.getElementById("new-notebook-confirm-btn").addEventListener("click", async () => {
  closeNewNotebookConfirmPanel();
  await invoke("new_notebook");
  await refresh();
});

// The gallery picker (Rodada Galeria): `gallery_list` is static (no
// AppState involved), so it's fetched once and cached -- reopening the
// panel never round-trips again. Each row calls `load_gallery` with the
// LAST block's id as the insertion point (or `undefined` for an empty
// notebook), so a loaded entry always lands at the end, never in the
// middle of existing work -- matches `Notebook::load_gallery_entry`'s
// own "leaves the rest of the notebook untouched" contract: appending
// after the end can never touch anything above it.
let galleryEntriesCache = null;

async function openGalleryPanel() {
  const panel = document.getElementById("gallery-panel");
  const list = document.getElementById("gallery-list");
  if (!galleryEntriesCache) {
    galleryEntriesCache = await invoke("gallery_list");
  }
  list.innerHTML = "";
  for (const entry of galleryEntriesCache) {
    const li = document.createElement("li");
    const btn = document.createElement("button");
    btn.className = "gallery-entry";
    btn.dataset.galleryName = entry.name;

    const title = document.createElement("div");
    title.className = "gallery-entry-title";
    title.textContent = entry.title;
    btn.appendChild(title);

    const description = document.createElement("div");
    description.className = "gallery-entry-description";
    description.textContent = entry.description;
    btn.appendChild(description);

    const invariant = document.createElement("div");
    invariant.className = "gallery-entry-invariant";
    invariant.textContent = entry.invariant;
    btn.appendChild(invariant);

    btn.addEventListener("click", () => loadGalleryEntry(entry.name));
    li.appendChild(btn);
    list.appendChild(li);
  }
  panel.hidden = false;
}

function closeGalleryPanel() {
  document.getElementById("gallery-panel").hidden = true;
}

async function loadGalleryEntry(name) {
  closeGalleryPanel();
  const { blocks } = await invoke("list_blocks");
  const after = blocks.length > 0 ? blocks[blocks.length - 1].id : undefined;
  let newIds;
  try {
    newIds = await invoke("load_gallery", { after, name });
  } catch (e) {
    window.alert("Erro ao carregar da galeria: " + e);
    return;
  }
  await refresh(newIds.length > 0 ? newIds[0] : undefined);
}

document.getElementById("gallery-btn").addEventListener("click", () => {
  const panel = document.getElementById("gallery-panel");
  if (panel.hidden) openGalleryPanel();
  else closeGalleryPanel();
});
document.getElementById("gallery-close-btn").addEventListener("click", closeGalleryPanel);
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  if (!document.getElementById("gallery-panel").hidden) closeGalleryPanel();
  // Escape only ever CLOSES the confirmation, exactly like Cancelar --
  // it must never be a shortcut for confirming a destructive action.
  if (!document.getElementById("new-notebook-confirm-panel").hidden) closeNewNotebookConfirmPanel();
});
// A click anywhere outside the panel or its own toggle button closes it
// -- standard dropdown behavior, and the reason this is a plain panel
// rather than a modal (no backdrop to catch this for free).
document.addEventListener("click", (e) => {
  const panel = document.getElementById("gallery-panel");
  if (!panel.hidden && !panel.contains(e.target) && !e.target.closest("#gallery-btn")) closeGalleryPanel();

  // Same convention for the "Novo caderno" confirmation -- clicking
  // away CANCELS, same as Escape/Cancelar, never confirms.
  const confirmPanel = document.getElementById("new-notebook-confirm-panel");
  if (!confirmPanel.hidden && !confirmPanel.contains(e.target) && !e.target.closest("#new-notebook-btn")) closeNewNotebookConfirmPanel();
});

window.addEventListener("load", async () => {
  // Headless-checkable proof this script itself ran -- kept permanently
  // from the Etapa 3a-2 toolchain proof, see src-tauri/src/lib.rs's
  // `frontend_ready` doc comment.
  await invoke("frontend_ready");
  const { blocks } = await invoke("list_blocks");
  // The first block gets real keyboard focus on open -- without this,
  // every key (not just Shift/Ctrl/Alt-Enter) needs a mouse click
  // first, since nothing has ever called `.focus()` on any editor yet.
  // Focusing is never executing: every block opens `NeverRun`, exactly
  // as before this change.
  await refresh(blocks.length > 0 ? blocks[0].id : undefined);
});
