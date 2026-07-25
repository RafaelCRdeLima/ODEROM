// Automated UI-layer test driver for the Jupyter-style execute keys
// (Shift/Ctrl/Alt-Enter) and real focus management -- DESIGN-NOTEBOOK.md
// Etapa 3a-2's key semantics. Loaded only by keytest.html, never by the
// real index.html a person uses.
//
// Deliberately does NOT call `execute_block`/`edit_block` or `cm.focus()`
// directly to simulate the things under test -- that would prove
// nothing about the keymap or focus wiring, which is exactly the layer
// that broke in an earlier round of this project's own session log.
// Every action below goes through a real DOM event dispatched at the
// element that would receive it in a genuine keypress/click, running
// through CodeMirror's own internal event handling unmodified.
//
// keyCode: CodeMirror 5's `keyName()` (codemirror.js, read directly
// rather than assumed) resolves a key purely via `keyNames[event.keyCode]`
// -- the legacy numeric field a real hardware keypress has always had
// populated by the browser/webview, but which the modern `KeyboardEvent`
// constructor does not let `key`/`code` populate. Forced via
// `Object.defineProperty` below, exactly reproducing what a genuine
// keypress already provides -- confirmed, not assumed, earlier in this
// project's own session log (a raw `new KeyboardEvent(...)` without
// this came out `keyCode: 0` and silently matched nothing).
//
// Every wait below polls a SPECIFIC, precise condition (e.g. "this
// exact block id's input field is document.activeElement") rather than
// "something is focused". The extraKeys handlers are `async`, so
// dispatching the keydown event only *starts* the edit_block ->
// execute_block -> refresh -> focus chain -- the block that was just
// executing is still focused for a little while after dispatch, and a
// weaker wait condition (e.g. "something, anything, is focused") reads
// that stale state as if it were the outcome. Caught exactly this
// mistake once while writing this file, from its own flaky first draft.
(async function () {
  const { invoke } = window.__TAURI__.core;
  const report = { phases: [] };

  function record(phase, ok, details) {
    report.phases.push({ phase, ok, details });
  }

  async function finish() {
    report.ok = report.phases.every((p) => p.ok);
    await invoke("ui_test_report", { message: JSON.stringify(report, null, 2) });
  }

  // `list_blocks` returns `{ blocks, current_path }` -- only `blocks`
  // matters to this test.
  async function listBlocks() {
    const { blocks } = await invoke("list_blocks");
    return blocks;
  }

  // `conditionFn` may be sync or `async` -- `Promise.resolve().then(conditionFn)`
  // handles either uniformly. A real, previously-latent bug found
  // writing this feature's own cancellation tests: the earlier version
  // called `conditionFn()` and checked *that* return value directly,
  // which for an async function is always a pending Promise object --
  // never `undefined`/`false` -- so it "resolved" (via `resolve()`
  // adopting the thenable, per Promise/A+) on the very first poll,
  // whatever the function eventually settled to, with the retry loop
  // below never actually running. Every async-conditionFn caller before
  // this feature happened to have its condition already true on the
  // first check (everything was synchronous), so this never surfaced
  // until a genuinely-async wait (a block still `Running` after the
  // first check) needed a real retry.
  function waitFor(conditionFn, timeoutMs = 20000) {
    return new Promise((resolve, reject) => {
      const deadline = Date.now() + timeoutMs;
      (function poll() {
        Promise.resolve()
          .then(conditionFn)
          .then(
            (result) => {
              // `0` is a valid block id, not "nothing found yet" --
              // only `undefined`/`false` mean "keep polling".
              if (result !== undefined && result !== false) return resolve(result);
              if (Date.now() > deadline) return reject(new Error("timed out waiting for a condition"));
              setTimeout(poll, 50);
            },
            () => {
              if (Date.now() > deadline) return reject(new Error("timed out waiting for a condition"));
              setTimeout(poll, 50);
            },
          );
      })();
    });
  }

  function focusedInputField() {
    const el = document.activeElement;
    return el && el.tagName === "TEXTAREA" && el.closest(".CodeMirror") ? el : null;
  }

  function blockIdOfInputField(el) {
    if (!el) return undefined;
    for (const id in editors) {
      if (editors[id].getInputField() === el) return Number(id);
    }
    return undefined;
  }

  // Waits for focus to land specifically away from `awayFromId` -- the
  // one reliable signal that an extraKeys handler's whole async chain
  // has actually finished, since focus is the LAST thing that chain
  // does and it always changes block. Used after Shift-Enter/Alt-Enter,
  // which always move focus somewhere new.
  function waitForFocusAwayFrom(awayFromId, timeoutMs = 20000) {
    return waitFor(() => {
      const f = blockIdOfInputField(focusedInputField());
      return f !== undefined && f !== awayFromId ? f : undefined;
    }, timeoutMs);
  }

  // Waits for a SPECIFIC block id's (possibly freshly-recreated)
  // CodeMirror instance to be the real, focused input field. Used after
  // Ctrl-Enter, which keeps focus on the same block id -- so "focus
  // moved away" is never true and cannot be used as the completion
  // signal there.
  function waitForFocusOn(blockId, timeoutMs = 20000) {
    return waitFor(() => editors[blockId] && document.activeElement === editors[blockId].getInputField(), timeoutMs);
  }

  // Etapa 3b (cancelamento): `execute_block` returns as soon as a
  // query's computation is handed off to its own thread, not once it
  // finishes -- a block can legitimately still be `Attempt{state:
  // "running"}` after every other sign an action "completed" (focus
  // moved, a new block appeared) is already true. Waits for `blockId`
  // to leave that state, whichever way it settles (a normal result, a
  // failure, or a cancellation all count as "no longer running" here;
  // callers that care which one check the DTO themselves afterward).
  function waitUntilSettled(blockId, timeoutMs = 20000) {
    return waitFor(async () => {
      const b = (await listBlocks()).find((x) => x.id === blockId);
      if (!b) return true; // deleted meanwhile -- nothing left to wait for
      const stillRunning = b.output.kind === "Attempt" && b.output.state === "running";
      return stillRunning ? undefined : true;
    }, timeoutMs);
  }

  function dispatchEnter(target, modifiers) {
    const ev = new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true, ...modifiers });
    Object.defineProperty(ev, "keyCode", { get: () => 13 });
    Object.defineProperty(ev, "which", { get: () => 13 });
    target.dispatchEvent(ev);
  }

  function clickToFocus(blockId) {
    const cm = editors[blockId];
    const line = cm.getWrapperElement().querySelector(".CodeMirror-line") || cm.getWrapperElement();
    for (const type of ["mousedown", "mouseup", "click"]) {
      line.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window }));
    }
  }

  function blockDiv(blockId) {
    return editors[blockId].getWrapperElement().closest(".block");
  }

  // Etapa 3b, segunda parte (exclusão mútua): `notebook.js`'s
  // `flashRefusal` stamps a monotonically increasing counter into
  // `dataset.refusalPulse` on both the status bar and the block
  // actually occupying execution, specifically so a refusal has a
  // real, timing-independent, assertable EDGE -- not just a check that
  // the final on-screen text happens to be correct, which would pass
  // even if the refused key had produced no visible reaction at all
  // (the status bar already said the same thing before the key was
  // pressed). `pulseOf`/`waitForNewPulse` read and wait on that same
  // counter.
  function pulseOf(el) {
    return el && el.dataset ? el.dataset.refusalPulse : undefined;
  }

  function waitForNewPulse(el, before, timeoutMs) {
    return waitFor(() => (pulseOf(el) !== before ? true : undefined), timeoutMs);
  }

  // Shared by Phase 4 (obsolescence) and Phase 5 (cancellation) below.
  // Deliberately does NOT call `editors[blockId].setValue(...)` -- that
  // fires a real CodeMirror "change" event, which schedules
  // notebook.js's own 400ms debounced `edit_block` sync on THIS (soon
  // to be destroyed) instance. `refresh()` below tears the instance
  // down before that timer fires, but the timer itself is never
  // cancelled -- it goes off later anyway, calls `edit_block` again
  // with this stale instance's captured text, and silently re-edits a
  // block the test has already moved past (found the hard way: it
  // corrupted a block's state several phases downstream in this exact
  // file, during this exact feature's own testing). Calling
  // `edit_block` directly is not a shortcut around that -- it is
  // *exactly* what the debounce timer itself would eventually do, just
  // without the stray timer.
  async function editViaRealEditor(blockId, newSource) {
    await invoke("edit_block", { id: blockId, source: newSource });
    await refresh();
  }

  // Shared by Phase 4 and Phase 5. Executes `blockId` via a real
  // dispatched Ctrl-Enter (keeps focus put, creates nothing -- the
  // cleanest of the three keys for setup) and waits for it to actually
  // *finish* (not just start) -- safe to use for a block expected to
  // settle quickly (a declaration, or a query against a metric known to
  // be fast). Phase 5's own long-running case does NOT use this: it
  // needs to observe the *running* state and stay there for a while, so
  // it dispatches Ctrl-Enter and waits directly, inline.
  //
  // `execution_count` is captured *before* dispatching and compared
  // against, not just "is it currently settled" -- the same mistake
  // this exact helper shipped once already, earlier in this feature's
  // own testing: checking only the current/final state is trivially,
  // spuriously true on the very first poll, before the dispatched
  // keydown's own async handler chain (edit_block -> execute_block ->
  // ...) has necessarily done anything at all.
  async function executeViaRealKeypress(blockId) {
    clickToFocus(blockId);
    await waitForFocusOn(blockId);
    const prevCount = (await listBlocks()).find((b) => b.id === blockId).execution_count;
    const target = editors[blockId].getInputField();
    dispatchEnter(target, { ctrlKey: true });
    await waitFor(async () => {
      const b = (await listBlocks()).find((x) => x.id === blockId);
      if (!b || b.execution_count === prevCount) return undefined;
      const stillRunning = b.output.kind === "Attempt" && b.output.state === "running";
      if (stillRunning) return undefined;
      // The backend settling and the frontend's own `pollUntilSettled`
      // loop noticing and updating the DOM are two independent polls on
      // two different intervals -- callers that immediately do DOM
      // work (e.g. Phase 4's delete-button click, found via
      // `editors[id]`'s own wrapper) need the gutter to actually show
      // the real number, not just the backend's own state.
      const div = blockDiv(blockId);
      const gutterText = div && div.querySelector(".block-gutter") && div.querySelector(".block-gutter").textContent;
      return gutterText === `[${b.execution_count}]` ? true : undefined;
    });
  }

  try {
    // Wait for the real page load handler (notebook.js) to finish its
    // own initial refresh() + auto-focus, not a fixed sleep.
    await waitFor(() => document.querySelector(".CodeMirror"));
    await waitFor(() => focusedInputField() !== null, 5000);

    const initialBlocks = await listBlocks();
    record("startup", initialBlocks.length === 4, { blockCount: initialBlocks.length });

    const focusedAtStart = blockIdOfInputField(focusedInputField());
    record("auto_focus_on_load", focusedAtStart === initialBlocks[0].id, { focusedBlockId: focusedAtStart, expected: initialBlocks[0].id });
    // Not just DOM focus -- the visible ring the redesign asks for
    // (".block.focused" in notebook.css) has to actually be there, not
    // just document.activeElement being technically correct.
    record("focused_block_shows_the_visual_ring", document.querySelectorAll(".block.focused").length === 1, {
      focusedClassCount: document.querySelectorAll(".block.focused").length,
      firstBlockClassList: document.querySelectorAll(".block")[0] && Array.from(document.querySelectorAll(".block")[0].classList),
    });

    // ---- Phase 1: four Shift+Enter in a row, no mouse ----
    let currentId = initialBlocks[0].id;
    const executedOrder = [];
    for (let i = 0; i < 4; i++) {
      const before = await listBlocks();
      const beforeCount = before.length;
      const executingId = currentId;
      const target = editors[executingId].getInputField();
      dispatchEnter(target, { shiftKey: true });

      const focusedNow = await waitForFocusAwayFrom(executingId);
      // Etapa 3b (cancelamento): `execute_block` now returns as soon as
      // a query's computation is handed off to its own thread, not
      // once it finishes -- focus already moved on to the next/new
      // block (confirmed above) while `executingId` can still be
      // showing `Attempt{state:"running"}`. Wait for it to actually
      // settle before reading its final output below, same as a real
      // person watching the gutter would.
      await waitUntilSettled(executingId);

      const after = await listBlocks();
      const executedBlock = after.find((b) => b.id === executingId);
      executedOrder.push({
        id: executingId,
        kind: executedBlock.output.kind,
        status: executedBlock.output.status || executedBlock.output.state,
        executionCount: executedBlock.execution_count,
      });

      const idxBefore = before.findIndex((b) => b.id === executingId);
      const wasLast = idxBefore === beforeCount - 1;
      if (wasLast) {
        record(`shift_enter_${i}_created_new_block`, after.length === beforeCount + 1, { beforeCount, afterCount: after.length });
      } else {
        record(`shift_enter_${i}_focus_moved_to_next`, focusedNow === before[idxBefore + 1].id, { focusedNow, expected: before[idxBefore + 1].id });
      }
      currentId = focusedNow;
    }

    record("all_four_blocks_executed_in_order", executedOrder.length === 4 && executedOrder.every((b) => b.kind !== "NeverRun"), { executedOrder });
    record(
      "execution_counts_are_sequential_and_unique",
      new Set(executedOrder.map((b) => b.executionCount)).size === 4 && executedOrder.every((b, i) => i === 0 || b.executionCount > executedOrder[i - 1].executionCount),
      { executedOrder },
    );

    const afterPhase1 = await listBlocks();
    record("fifth_block_created", afterPhase1.length === 5, { count: afterPhase1.length });
    const fifthBlock = afterPhase1[4];
    record("fifth_block_is_empty_and_never_run", fifthBlock.source === "" && fifthBlock.output.kind === "NeverRun", { fifthBlock });
    record("fifth_block_has_no_execution_count", fifthBlock.execution_count === null || fifthBlock.execution_count === undefined, { executionCount: fifthBlock.execution_count });

    const kretschmannBlock = afterPhase1[3];
    const kretschmannHasLatex = kretschmannBlock.output.kind === "Query" && !!kretschmannBlock.output.latex;
    record("kretschmann_output_present", kretschmannHasLatex, { output: kretschmannBlock.output });

    // DOM check, not just the DTO: the fourth block's output element
    // should actually contain a rendered KaTeX node, and its gutter
    // should show the real execution number, not a placeholder.
    //
    // Etapa 3b (cancelamento): the backend settling (just confirmed via
    // `kretschmann_output_present`, backed by `waitUntilSettled` polling
    // `list_blocks` directly) and the frontend's own `pollUntilSettled`
    // loop (`notebook.js`) noticing and updating the DOM are two
    // independent polls on two different intervals -- the backend can
    // easily be observed as "done" up to one of the frontend's own poll
    // cycles (150ms) before the DOM actually reflects it. Waiting on the
    // DOM itself here, not just re-trusting the earlier backend check,
    // is what makes this a real DOM assertion instead of an assumption.
    await waitFor(() => document.querySelector(".block:nth-child(4) .katex") || undefined);
    const blockDivs = document.querySelectorAll(".block");
    const fourthOutputEl = blockDivs[3] && blockDivs[3].querySelector(".block-output");
    const domHasKatex = !!(fourthOutputEl && fourthOutputEl.querySelector(".katex"));
    record("kretschmann_output_in_dom", domHasKatex, { outputHtmlSnippet: fourthOutputEl ? fourthOutputEl.innerHTML.slice(0, 200) : null });
    const fourthGutterText = blockDivs[3] && blockDivs[3].querySelector(".block-gutter") && blockDivs[3].querySelector(".block-gutter").textContent;
    record("kretschmann_gutter_shows_a_real_number", /^\[\d+\]$/.test(fourthGutterText || ""), { gutterText: fourthGutterText });
    const fifthGutterText = blockDivs[4] && blockDivs[4].querySelector(".block-gutter") && blockDivs[4].querySelector(".block-gutter").textContent;
    record("fifth_gutter_shows_empty_marker", fifthGutterText === "[ ]", { gutterText: fifthGutterText });

    record("fifth_block_has_real_focus", blockIdOfInputField(focusedInputField()) === fifthBlock.id, { focused: blockIdOfInputField(focusedInputField()), expected: fifthBlock.id });

    // ---- Phase 2: Ctrl+Enter -- focus stays, nothing created ----
    {
      const before = await listBlocks();
      const id = fifthBlock.id;
      const target = editors[id].getInputField();
      dispatchEnter(target, { ctrlKey: true });
      await waitFor(async () => {
        const bs = await listBlocks();
        const b = bs.find((x) => x.id === id);
        return b && b.output.kind !== "NeverRun";
      });
      await waitForFocusOn(id); // itself the precise proof focus stayed put
      const after = await listBlocks();
      record("ctrl_enter_creates_nothing", after.length === before.length, { before: before.length, after: after.length });
      record("ctrl_enter_keeps_focus_on_same_block", true, { focused: id });
    }

    // ---- Phase 3: Alt+Enter in the middle -- new block inserted
    // between two existing ones, focus on the new one ----
    {
      const before = await listBlocks();
      const middleId = before[1].id; // the "chart" block -- safely mid-notebook
      clickToFocus(middleId);
      await waitForFocusOn(middleId);
      const target = editors[middleId].getInputField();
      dispatchEnter(target, { altKey: true });

      const newBlockId = await waitForFocusAwayFrom(middleId);

      const after = await listBlocks();
      const middleIdxAfter = after.findIndex((b) => b.id === middleId);
      const newBlock = after[middleIdxAfter + 1];
      const isBetween = newBlock && newBlock.id === newBlockId && after.length === before.length + 1;
      record("alt_enter_inserts_between_existing_blocks", !!isBetween, {
        before: before.map((b) => b.id),
        after: after.map((b) => b.id),
      });
      record("alt_enter_focuses_new_block", blockIdOfInputField(focusedInputField()) === newBlockId, { focused: blockIdOfInputField(focusedInputField()), expected: newBlockId });
    }

    // ---- Phase 4: obsolescence (Etapa 3b, DESIGN-NOTEBOOK.md section 9) ----
    // A dedicated scenario, appended entirely at the end -- isolated
    // from every block Phases 1-3 already touched, since the
    // conservative cascade rule only ever marks blocks *below* an
    // edit's position, and every block created here starts out below
    // everything above. Names (ObsM/ObsTM/obsChart/obsG) are distinct
    // from the seed notebook's own (M/TM/schw/g) so reconstruction,
    // which concatenates *every* declaration block in the whole
    // notebook, never trips a duplicate-name error because of it.
    //
    // Executing goes through a real dispatched Ctrl-Enter (keeps focus
    // put, creates nothing -- the cleanest of the three for setup,
    // exact same mechanism Phase 2 already proved reliable). Editing
    // goes through the real CodeMirror instance's own `setValue` (a
    // genuine CodeMirror change event, the same pipeline a paste would
    // drive) followed by the exact `edit_block` call the debounced
    // "change" handler would itself make -- not waiting the debounce's
    // own 400ms is a timing shortcut, not a different code path.
    // Deleting goes through a real dispatched click on the real delete
    // button. This phase is about the *display* of obsolescence, not
    // about keymap/focus wiring -- already Phases 1-3's job -- so
    // driving edits/creates through `invoke` where there is no keyboard
    // gesture to speak of (e.g. `create_block`, which the real UI only
    // ever calls as a side effect of a keypress already exercised
    // above) keeps this phase focused without retesting the same
    // wiring twice.
    {
      const OBS_A = "manifold ObsM dim 4\nbundle ObsTM on ObsM dim 4";
      const OBS_B = "chart obsChart on ObsM coords (t, r, theta, phi)";
      const OBS_C = "metric obsG on obsChart bundle ObsTM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const OBS_Q = "ricci";

      const oA = await invoke("create_block", { after: null, source: OBS_A });
      const oB = await invoke("create_block", { after: oA, source: OBS_B });
      const oC = await invoke("create_block", { after: oB, source: OBS_C });
      const oQ = await invoke("create_block", { after: oC, source: OBS_Q });
      await refresh();

      await executeViaRealKeypress(oA);
      await executeViaRealKeypress(oB);
      await executeViaRealKeypress(oC);
      await executeViaRealKeypress(oQ);

      const setup = await listBlocks();
      record("obsolescence_setup_all_four_executed_and_not_obsolete", [oA, oB, oC, oQ].every((id) => !setup.find((b) => b.id === id).obsolete), {
        obsolete: [oA, oB, oC, oQ].map((id) => setup.find((b) => b.id === id).obsolete),
      });

      // ---- Criterion 1: editing oB marks oB/oC/oQ obsolete, oA
      // untouched, and triggers no execution -- oQ's own displayed
      // result must be byte-for-byte the same as before the edit. ----
      const qOutputBefore = setup.find((b) => b.id === oQ).output;

      // A trivial, always-valid text change (trailing newline) rather
      // than a coordinate rename -- this phase's own assertions only
      // ever check obsolete flags, never reconstruction success, so a
      // rename that renamed the chart's coordinate list but not the
      // metric's own `[phi,phi]` bracket reference to match (a real,
      // separate bug: `_obs` also wasn't even a valid identifier
      // character, `_` lexes as its own reserved token for tensor-index
      // subscripts) went unnoticed here -- until Phase 5 needed the
      // *whole* declaration set to build cleanly and it silently didn't.
      await editViaRealEditor(oB, OBS_B + "\n");

      const afterEdit = await listBlocks();
      const flags = (bs, id) => bs.find((b) => b.id === id).obsolete;
      record(
        "edit_marks_self_and_everything_below_obsolete",
        flags(afterEdit, oB) && flags(afterEdit, oC) && flags(afterEdit, oQ) && !flags(afterEdit, oA),
        { oA: flags(afterEdit, oA), oB: flags(afterEdit, oB), oC: flags(afterEdit, oC), oQ: flags(afterEdit, oQ) },
      );
      record("edit_triggers_no_execution_output_byte_identical", JSON.stringify(afterEdit.find((b) => b.id === oQ).output) === JSON.stringify(qOutputBefore), {
        before: qOutputBefore,
        after: afterEdit.find((b) => b.id === oQ).output,
      });

      // DOM check, not just the DTO -- the real amber class, the real
      // stripe/gutter marker's presence, and the real "resultado
      // obsoleto" text label.
      const oQDiv = blockDiv(oQ);
      record("obsolete_block_shows_amber_class_and_label_in_dom", oQDiv.classList.contains("obsolete") && !!oQDiv.querySelector(".obsolete-label"), {
        classList: Array.from(oQDiv.classList),
        labelText: oQDiv.querySelector(".obsolete-label") && oQDiv.querySelector(".obsolete-label").textContent,
      });

      // ---- Criterion 2: reexecuting oB clears only oB's own mark ----
      await executeViaRealKeypress(oB);
      const afterReexec = await listBlocks();
      record(
        "reexecuting_clears_only_that_blocks_own_mark",
        !flags(afterReexec, oB) && flags(afterReexec, oC) && flags(afterReexec, oQ),
        { oB: flags(afterReexec, oB), oC: flags(afterReexec, oC), oQ: flags(afterReexec, oQ) },
      );

      // ---- Criterion 3: undo (edit back to byte-identical text)
      // clears the mark -- a dedicated, isolated block (created last,
      // nothing below it, nothing above it edited again from here on)
      // so this can't be contaminated by the cascade mark oC/oQ still
      // legitimately carry from criterion 1. ----
      const oU = await invoke("create_block", { after: null, source: "scalar" });
      await refresh();
      await executeViaRealKeypress(oU);
      const oUOutputBefore = (await listBlocks()).find((b) => b.id === oU).output;

      await editViaRealEditor(oU, "scalar\n");
      record("undo_setup_edit_marks_it_obsolete", flags(await listBlocks(), oU), {});

      await editViaRealEditor(oU, "scalar"); // back to byte-identical with what was executed
      const afterUndo = (await listBlocks()).find((b) => b.id === oU);
      record("undoing_edit_back_to_executed_text_clears_the_mark", !afterUndo.obsolete, { obsolete: afterUndo.obsolete });
      record("undo_did_not_change_the_result", JSON.stringify(afterUndo.output) === JSON.stringify(oUOutputBefore), { before: oUOutputBefore, after: afterUndo.output });

      // ---- Criterion 4: deleting a block in the middle marks only the
      // executed blocks below it, never the ones above ----
      const dA = await invoke("create_block", { after: null, source: "scalar" });
      const dB = await invoke("create_block", { after: dA, source: "scalar" });
      const dC = await invoke("create_block", { after: dB, source: "scalar" });
      await refresh();
      await executeViaRealKeypress(dA);
      await executeViaRealKeypress(dB);
      await executeViaRealKeypress(dC);

      const deleteBtn = blockDiv(dB).querySelector(".block-delete");
      deleteBtn.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(async () => ((await listBlocks()).find((b) => b.id === dB) ? undefined : true));

      const afterDelete = await listBlocks();
      record("deleting_marks_only_executed_blocks_below_it_obsolete", !flags(afterDelete, dA) && flags(afterDelete, dC), {
        dA: flags(afterDelete, dA),
        dC: flags(afterDelete, dC),
      });

      // ---- Criterion 5: a block never itself directly executed can
      // never be obsolete, no matter how it's edited -- stays `[ ]` ----
      const nA = await invoke("create_block", { after: null, source: "manifold NeverExec dim 2" });
      await refresh();
      await editViaRealEditor(nA, "manifold NeverExec dim 3");
      const neverExecuted = (await listBlocks()).find((b) => b.id === nA);
      record("a_never_executed_block_cannot_be_obsolete", !neverExecuted.obsolete && neverExecuted.execution_count == null, { neverExecuted });

      const nADiv = blockDiv(nA);
      const nAGutterText = nADiv.querySelector(".block-gutter").textContent;
      record("never_executed_block_gutter_shows_empty_marker_not_amber", nAGutterText === "[ ]" && !nADiv.classList.contains("obsolete"), { gutterText: nAGutterText });
    }

    // ---- Phase 5: cancellation (Etapa 3b, DESIGN-NOTEBOOK.md) ----
    // A dedicated scenario, appended entirely at the end, same
    // isolation reasoning as Phase 4. Uses this project's own standing
    // non-terminating-computation fixture
    // (`oderom-session/tests/cancellation.rs`, the exact metric shape
    // that produced the real freeze this feature exists to fix) --
    // never an artificial `sleep`, so cancelling it genuinely proves
    // the worker thread stops, not just that a timer elapsed. The
    // query names its metric explicitly (`kretschmann cancelG`, not a
    // bare `kretschmann`) since by this point several *other* metrics
    // already exist in the live model (the seed's own, Phase 4's
    // `obsG`) and a bare query would hit "more than one metric" instead
    // of actually running.
    {
      const CANCEL_DECLS = "manifold CancelM dim 4\nbundle CancelTM on CancelM dim 4\nchart cancelChart on CancelM coords (t, r, theta, phi)";
      const FAST_METRIC =
        CANCEL_DECLS +
        "\nmetric cancelG on cancelChart bundle CancelTM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const SLOW_METRIC =
        CANCEL_DECLS +
        "\nmetric cancelG on cancelChart bundle CancelTM {\n  [t,t] = -(1 - 2*M/r + 1/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const QUERY = "kretschmann cancelG";

      // Dispatches a real Ctrl-Enter on `blockId` and waits only for it
      // to *start* (execution_count bumped, DTO shows Attempt/running)
      // -- unlike `executeViaRealKeypress`, never waits for it to
      // finish, since for this block it deliberately won't for a long
      // time.
      async function startViaRealKeypress(blockId) {
        clickToFocus(blockId);
        await waitForFocusOn(blockId);
        const prevCount = (await listBlocks()).find((b) => b.id === blockId).execution_count;
        dispatchEnter(editors[blockId].getInputField(), { ctrlKey: true });
        await waitFor(async () => {
          const b = (await listBlocks()).find((x) => x.id === blockId);
          return b && b.execution_count !== prevCount && b.output.kind === "Attempt" && b.output.state === "running" ? true : undefined;
        });
      }

      const cA = await invoke("create_block", { after: null, source: FAST_METRIC });
      const cOther = await invoke("create_block", { after: cA, source: "scalar cancelG" }); // deliberately left never-executed -- criterion 6 checks it stays that way
      const cQ = await invoke("create_block", { after: cOther, source: QUERY });
      await refresh();

      await executeViaRealKeypress(cA);
      await executeViaRealKeypress(cQ); // fast -- a real, successful result to later find preserved

      // Edit the metric to the non-reciprocal shape and reexecute it
      // (fast -- reconstruction only, no curvature), then start `cQ`
      // again -- THIS run hangs.
      await editViaRealEditor(cA, SLOW_METRIC);
      await executeViaRealKeypress(cA);
      await startViaRealKeypress(cQ);

      // Give it real time inside the stage that used to hang for 60+s
      // (same 300ms `oderom-session/tests/cancellation.rs` itself uses)
      // -- long enough to be genuinely stuck deep inside a single
      // component's `normalize()` call, not just between components.
      await new Promise((resolve) => setTimeout(resolve, 300));

      // ---- Criterion 1: the window stays responsive while it runs --
      // edit and focus a DIFFERENT block for real, bounded by waitFor's
      // own timeout. If the app were actually frozen (the bug this
      // feature exists to fix), these would time out and fail here,
      // not silently pass. ----
      await editViaRealEditor(cOther, "riemann cancelG");
      const otherAfterEdit = (await listBlocks()).find((b) => b.id === cOther);
      record("window_stays_responsive_edit_reaches_another_block_while_a_long_execution_runs", otherAfterEdit.source === "riemann cancelG", {
        source: otherAfterEdit.source,
      });

      clickToFocus(cA);
      await waitForFocusOn(cA);
      record("window_stays_responsive_can_focus_another_block_while_a_long_execution_runs", blockIdOfInputField(focusedInputField()) === cA, {});

      // ---- Criterion 2: cancel it for real, and prove the worker
      // thread actually stopped, not just that the UI changed ----
      const cancelBtnPresentWhileRunning = !!blockDiv(cQ).querySelector(".block-cancel");
      blockDiv(cQ).querySelector(".block-cancel").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      // The DTO leaving "running" can only happen from inside
      // `Notebook::finish_query`, reachable only *after*
      // `PendingQuery::run()` -- and so `run_cancellable`'s own
      // catch_unwind -- has returned. That is the proof the thread
      // stopped, not merely that some UI class changed.
      await waitFor(async () => {
        const b = (await listBlocks()).find((x) => x.id === cQ);
        return b && !(b.output.kind === "Attempt" && b.output.state === "running") ? true : undefined;
      });
      const afterCancel = (await listBlocks()).find((b) => b.id === cQ);
      record("cancel_button_is_present_while_running", cancelBtnPresentWhileRunning, {});
      record("cancelling_actually_stops_the_computation", afterCancel.output.kind === "Attempt" && afterCancel.output.state === "cancelled", { output: afterCancel.output });

      // Etapa 3b's `pollUntilSettled` (notebook.js) is what a real user
      // relies on to see this update with no action of their own -- but
      // *this* test already has independent, stronger proof the backend
      // settled (the wait above, against `list_blocks` directly), so an
      // explicit `refresh()` here is the most direct way to assert what
      // the DOM *should* show once looked at, without also re-asserting
      // exactly how fast the background poll gets there on its own (a
      // real thing worth knowing, but a separate, timing-sensitive
      // question from "is cancellation itself correct end to end" --
      // the same reasoning `executeViaRealKeypress`'s own gutter-text
      // check already applies elsewhere in this file).
      await refresh();

      // ---- Criterion 4: the earlier result stays visible, marked
      // obsolete, in the real DOM ----
      const cQOutputEl = blockDiv(cQ).querySelector(".block-output");
      record("cancelled_block_with_a_previous_result_keeps_it_visible_and_marked_obsolete", !!cQOutputEl.querySelector(".katex") && !!cQOutputEl.querySelector(".obsolete-label"), {
        html: cQOutputEl.innerHTML.slice(0, 300),
      });
      record("cancelled_block_shows_the_cancelled_label_and_class", blockDiv(cQ).classList.contains("cancelled") && !!cQOutputEl.querySelector(".cancelled-label"), {
        classList: Array.from(blockDiv(cQ).classList),
      });

      // ---- Criterion 6: cancelling cQ did not execute cOther ----
      const cOtherFinal = (await listBlocks()).find((b) => b.id === cOther);
      record("cancelling_did_not_trigger_execution_of_any_other_block", cOtherFinal.execution_count == null && cOtherFinal.output.kind === "NeverRun", { cOtherFinal });

      // ---- Criterion 3: a block cancelled on its first-ever execution
      // shows cancelled with no output at all ----
      const cFresh = await invoke("create_block", { after: null, source: QUERY });
      await refresh();
      await startViaRealKeypress(cFresh);
      await new Promise((resolve) => setTimeout(resolve, 300));
      blockDiv(cFresh).querySelector(".block-cancel").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(async () => {
        const b = (await listBlocks()).find((x) => x.id === cFresh);
        return b && !(b.output.kind === "Attempt" && b.output.state === "running") ? true : undefined;
      });
      const cFreshAfter = (await listBlocks()).find((b) => b.id === cFresh);
      record(
        "cancelling_a_first_ever_execution_leaves_cancelled_state_with_no_previous",
        cFreshAfter.output.kind === "Attempt" && cFreshAfter.output.state === "cancelled" && cFreshAfter.output.previous === null,
        { output: cFreshAfter.output },
      );
      const cFreshOutputEl = blockDiv(cFresh).querySelector(".block-output");
      record("first_ever_cancelled_block_dom_shows_no_result_content", !cFreshOutputEl.querySelector(".katex") && !cFreshOutputEl.querySelector(".obsolete-label"), {
        html: cFreshOutputEl.innerHTML.slice(0, 300),
      });

      // ---- Criterion 5: reexecuting a cancelled block completes
      // normally and clears the cancelled state ----
      await editViaRealEditor(cA, FAST_METRIC); // revert -- fast again
      await executeViaRealKeypress(cA);
      await executeViaRealKeypress(cQ); // fast now -- completes normally, no cancellation this time
      const cQFinal = (await listBlocks()).find((b) => b.id === cQ);
      record("reexecuting_a_cancelled_block_completes_normally_and_clears_the_cancelled_state", cQFinal.output.kind === "Query" && !!cQFinal.output.latex, {
        output: cQFinal.output,
      });
    }
    // ---- Phase 6: mutual exclusion (Etapa 3b, segunda parte,
    // DESIGN-NOTEBOOK.md section 10.8) ----
    // A dedicated scenario, appended entirely at the end, same
    // isolation reasoning as Phases 4/5. Own declaration names
    // (BlockM/BlockTM/blockChart/blockG) distinct from every earlier
    // phase's own (M/TM/schw/g, ObsM/..., CancelM/...) so reconstruction
    // -- which concatenates every declaration block in the whole
    // notebook -- never trips a duplicate-name error.
    {
      const BLOCK_DECLS = "manifold BlockM dim 4\nbundle BlockTM on BlockM dim 4\nchart blockChart on BlockM coords (t, r, theta, phi)";
      const FAST_METRIC = BLOCK_DECLS + "\nmetric blockG on blockChart bundle BlockTM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const SLOW_METRIC = BLOCK_DECLS + "\nmetric blockG on blockChart bundle BlockTM {\n  [t,t] = -(1 - 2*M/r + 1/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";

      // Same shape as Phase 5's own local helper: dispatches a real
      // Ctrl-Enter and waits only for the execution to *start*, never
      // for it to finish.
      async function startViaRealKeypress(blockId) {
        clickToFocus(blockId);
        await waitForFocusOn(blockId);
        const prevCount = (await listBlocks()).find((b) => b.id === blockId).execution_count;
        dispatchEnter(editors[blockId].getInputField(), { ctrlKey: true });
        await waitFor(async () => {
          const b = (await listBlocks()).find((x) => x.id === blockId);
          return b && b.execution_count !== prevCount && b.output.kind === "Attempt" && b.output.state === "running" ? true : undefined;
        });
      }

      const eA = await invoke("create_block", { after: null, source: FAST_METRIC });
      const eQ1 = await invoke("create_block", { after: eA, source: "kretschmann blockG" });
      const eQ2 = await invoke("create_block", { after: eQ1, source: "riemann blockG" });
      await refresh();

      await executeViaRealKeypress(eA);
      // A real, successful previous result for eQ2 -- criterion (b)
      // needs something genuine to prove a refusal never touches.
      await executeViaRealKeypress(eQ2);

      // Switch to the non-reciprocal shape (this project's own standing
      // non-terminating-computation fixture) so eQ1 can be made to run
      // indefinitely -- criteria (a)/(b)/(c) need a block that is
      // reliably still running for as long as the test needs it to be,
      // not racing against how fast a real computation happens to
      // finish.
      await editViaRealEditor(eA, SLOW_METRIC);
      await executeViaRealKeypress(eA); // fast -- reconstruction only
      await startViaRealKeypress(eQ1); // this run hangs

      // Real time inside the stage that used to hang for 60+s (same
      // 300ms `oderom-session/tests/cancellation.rs`/Phase 5 already
      // use) -- long enough to be genuinely stuck deep inside a single
      // component's normalize() call, not just between components.
      await new Promise((resolve) => setTimeout(resolve, 300));

      const runningExecCount = (await listBlocks()).find((b) => b.id === eQ1).execution_count;
      const eQ2BeforeRefusal = (await listBlocks()).find((b) => b.id === eQ2);

      // ---- Criterion (a): Shift+Enter on a DIFFERENT block does not
      // start execution while eQ1 is running, and the refusal is a
      // real, perceptible EVENT -- not merely correct final text. The
      // status bar's busy message is ambient: it was already showing
      // before this key is pressed and looks identical whether or not
      // the refusal actually happened, so asserting only its final
      // content would pass even for a silently-swallowed keypress --
      // exactly the failure mode being guarded against here. What is
      // captured instead is the state immediately BEFORE the keydown
      // (`dataset.refusalPulse`, notebook.js's own `flashRefusal`
      // counter) and confirmed to have changed after it -- a real edge
      // at the instant of the key, the same one a person would see as
      // a flash. ----
      const statusEl = document.getElementById("status-left");
      const busyWrapperBeforeRefusal = blockDiv(eQ1);
      const statusPulseBefore = pulseOf(statusEl);
      const busyPulseBefore = pulseOf(busyWrapperBeforeRefusal);

      clickToFocus(eQ2);
      await waitForFocusOn(eQ2);
      const target = editors[eQ2].getInputField();
      dispatchEnter(target, { shiftKey: true });
      await waitForNewPulse(statusEl, statusPulseBefore);

      const statusPulseAfter = pulseOf(statusEl);
      record("the_refusal_produces_a_status_bar_event_distinct_from_the_unchanged_ambient_text", statusPulseAfter !== statusPulseBefore, {
        before: statusPulseBefore,
        after: statusPulseAfter,
      });
      const busyWrapperAfterRefusal = blockDiv(eQ1);
      const busyPulseAfter = pulseOf(busyWrapperAfterRefusal);
      record(
        "the_refusal_also_flashes_the_specific_block_that_is_occupying_execution",
        busyPulseAfter !== busyPulseBefore && busyPulseAfter === statusPulseAfter,
        { before: busyPulseBefore, after: busyPulseAfter, statusPulse: statusPulseAfter },
      );
      record("the_status_bar_text_also_correctly_names_the_busy_block_by_number", document.getElementById("status-left").textContent.includes(`[${runningExecCount}]`), {
        statusText: document.getElementById("status-left").textContent,
      });

      const stillRunningAfterShiftEnter = (await listBlocks()).find((b) => b.id === eQ1);
      record(
        "shift_enter_on_a_different_block_does_not_start_execution_while_one_is_running",
        stillRunningAfterShiftEnter.output.kind === "Attempt" && stillRunningAfterShiftEnter.output.state === "running" && stillRunningAfterShiftEnter.execution_count === runningExecCount,
        { output: stillRunningAfterShiftEnter.output },
      );
      record("blocked_shift_enter_does_not_move_focus_away_from_the_refused_block", blockIdOfInputField(focusedInputField()) === eQ2, {
        focused: blockIdOfInputField(focusedInputField()),
      });

      // The user's own requirement names all three execute keys
      // explicitly, not only Shift-Enter -- Alt-Enter (would otherwise
      // insert a new block) and Ctrl-Enter (would otherwise execute in
      // place) must be refused exactly the same way, each producing its
      // own new pulse (not just a lack of change).
      const blockCountBeforeAltEnter = (await listBlocks()).length;
      const statusPulseBeforeAlt = pulseOf(statusEl);
      dispatchEnter(target, { altKey: true });
      await waitForNewPulse(statusEl, statusPulseBeforeAlt);
      const blockCountAfterAltEnter = (await listBlocks()).length;
      record("alt_enter_on_a_different_block_is_also_refused_while_one_is_running", blockCountAfterAltEnter === blockCountBeforeAltEnter, {
        before: blockCountBeforeAltEnter,
        after: blockCountAfterAltEnter,
      });

      const statusPulseBeforeCtrl = pulseOf(statusEl);
      dispatchEnter(target, { ctrlKey: true });
      await waitForNewPulse(statusEl, statusPulseBeforeCtrl);
      const eQ2AfterCtrlEnter = (await listBlocks()).find((b) => b.id === eQ2);
      record("ctrl_enter_on_a_different_block_is_also_refused_while_one_is_running", eQ2AfterCtrlEnter.execution_count === eQ2BeforeRefusal.execution_count, {
        before: eQ2BeforeRefusal.execution_count,
        after: eQ2AfterCtrlEnter.execution_count,
      });

      // ---- Criterion (b): the refused block never changed state and
      // never lost its earlier (real, successful) output. ----
      const eQ2AfterAllRefusals = (await listBlocks()).find((b) => b.id === eQ2);
      record("refused_block_execution_count_stays_unchanged_across_every_refusal", eQ2AfterAllRefusals.execution_count === eQ2BeforeRefusal.execution_count, {
        before: eQ2BeforeRefusal.execution_count,
        after: eQ2AfterAllRefusals.execution_count,
      });
      record("refused_block_keeps_its_previous_output_byte_identical", JSON.stringify(eQ2AfterAllRefusals.output) === JSON.stringify(eQ2BeforeRefusal.output), {
        before: eQ2BeforeRefusal.output,
        after: eQ2AfterAllRefusals.output,
      });

      // ---- Criterion (e): editing and focus keep working on OTHER
      // blocks the whole time eQ1 is running -- blocking execution must
      // never block the interface. ----
      await editViaRealEditor(eQ2, "riemann blockG" + "\n"); // trivial, always-valid edit -- same pattern Phase 4 uses
      const eQ2AfterEdit = (await listBlocks()).find((b) => b.id === eQ2);
      record("editing_a_different_block_still_works_while_one_is_running", eQ2AfterEdit.source === "riemann blockG\n", { source: eQ2AfterEdit.source });

      clickToFocus(eA);
      await waitForFocusOn(eA);
      record("focusing_a_different_block_still_works_while_one_is_running", blockIdOfInputField(focusedInputField()) === eA, {});

      // ---- Criterion (c): cancelling the running block unblocks the
      // previously refused one, which now executes normally. ----
      blockDiv(eQ1).querySelector(".block-cancel").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(async () => {
        const b = (await listBlocks()).find((x) => x.id === eQ1);
        return b && !(b.output.kind === "Attempt" && b.output.state === "running") ? true : undefined;
      });
      const eQ1AfterCancel = (await listBlocks()).find((b) => b.id === eQ1);
      record("cancelling_the_running_block_settles_it_to_cancelled", eQ1AfterCancel.output.kind === "Attempt" && eQ1AfterCancel.output.state === "cancelled", {
        output: eQ1AfterCancel.output,
      });

      // `eQ2` ("riemann blockG") is still evaluated against
      // `SLOW_METRIC` at this point -- unblocking it here does not by
      // itself mean it will *finish* quickly (Riemann on the
      // non-reciprocal shape can itself be expensive), so the metric is
      // reverted to the reciprocal one first. This also sets up exactly
      // what criterion (d) below needs: `eA` back on `FAST_METRIC`.
      await editViaRealEditor(eA, FAST_METRIC);
      await executeViaRealKeypress(eA); // fast -- reconstruction only

      await executeViaRealKeypress(eQ2); // previously refused -- must now genuinely run and complete
      const eQ2AfterCancelUnblock = (await listBlocks()).find((b) => b.id === eQ2);
      record("cancelling_the_running_block_unblocks_the_previously_refused_block", eQ2AfterCancelUnblock.output.kind === "Query" && !!eQ2AfterCancelUnblock.output.latex, {
        output: eQ2AfterCancelUnblock.output,
      });

      // ---- Criterion (d): the running block finishing ON ITS OWN
      // (never cancelled) unblocks a refused block just as reliably.
      // Reuses this same session's own established finding that a
      // "fast" reciprocal-metric kretschmann genuinely takes several
      // real seconds under this sandbox's variable CPU load (the reason
      // `waitFor`'s own default timeout was bumped to 20000ms), which
      // is exactly the kind of real, non-instant, but finite window
      // this criterion needs -- a real computation, not an artificial
      // sleep: long enough that a keypress dispatched right after it
      // starts reliably still finds it running, short enough to finish
      // within this test's own deadline without being cancelled. ----
      await startViaRealKeypress(eQ1); // real, finite, multi-second computation -- not cancelled this time

      const eQ2BeforeSecondRefusal = (await listBlocks()).find((b) => b.id === eQ2);
      clickToFocus(eQ2);
      await waitForFocusOn(eQ2);
      const statusPulseBeforeSecondRefusal = pulseOf(statusEl);
      dispatchEnter(editors[eQ2].getInputField(), { ctrlKey: true });
      await waitForNewPulse(statusEl, statusPulseBeforeSecondRefusal);
      const eQ2DuringSecondRefusal = (await listBlocks()).find((b) => b.id === eQ2);
      record("a_block_started_right_before_a_natural_finish_is_still_refused_meanwhile", eQ2DuringSecondRefusal.execution_count === eQ2BeforeSecondRefusal.execution_count, {
        before: eQ2BeforeSecondRefusal.execution_count,
        after: eQ2DuringSecondRefusal.execution_count,
      });

      await waitUntilSettled(eQ1); // no cancel click here -- genuinely finishes on its own
      const eQ1AfterNaturalFinish = (await listBlocks()).find((b) => b.id === eQ1);
      record("the_running_block_settles_to_a_real_result_not_cancelled_when_left_to_finish_naturally", eQ1AfterNaturalFinish.output.kind === "Query" && !!eQ1AfterNaturalFinish.output.latex, {
        output: eQ1AfterNaturalFinish.output,
      });

      await executeViaRealKeypress(eQ2); // must now genuinely run, unblocked by the natural finish above
      const eQ2AfterNaturalUnblock = (await listBlocks()).find((b) => b.id === eQ2);
      record(
        "the_running_block_finishing_naturally_also_unblocks_the_previously_refused_block",
        eQ2AfterNaturalUnblock.output.kind === "Query" && !!eQ2AfterNaturalUnblock.output.latex,
        { output: eQ2AfterNaturalUnblock.output },
      );
    }
    // ---- Phase 7: expression aliases (NOME := EXPR, oderom-cli's own
    // parser) ----
    // A dedicated scenario, appended entirely at the end, same isolation
    // reasoning as Phases 4-6. Own declaration names (AliasM/AliasTM/
    // aliasChart) distinct from every earlier phase's own, for the same
    // reconstruction-concatenation reason those phases already document.
    // Deliberately spans TWO separate blocks (the alias declaration and
    // the metric that uses it) -- the whole point being tested is that
    // the alias's scope really is "notebook-wide from the declaration
    // point down", carried by the SAME obsolescence cascade every other
    // declaration already uses (`oderom-notebook`'s own
    // `editing_an_alias_block_marks_a_later_block_that_uses_it_obsolete`
    // test already covers this at the API level; this is the same
    // invariant through real keystrokes and a real DOM check).
    {
      const ALIAS_DECL = "falias := 1 - 2*M/r";
      const ALIAS_DECLS_REST = "manifold AliasM dim 4\nbundle AliasTM on AliasM dim 4\nchart aliasChart on AliasM coords (t, r, theta, phi)";
      const ALIAS_METRIC = "metric aliasG on aliasChart bundle AliasTM {\n  [t,t] = -falias,\n  [r,r] = 1/falias,\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const ALIAS_QUERY = "kretschmann aliasG";

      const aliasBlock = await invoke("create_block", { after: null, source: ALIAS_DECL });
      const aliasDeclsRest = await invoke("create_block", { after: aliasBlock, source: ALIAS_DECLS_REST });
      const aliasMetric = await invoke("create_block", { after: aliasDeclsRest, source: ALIAS_METRIC });
      const aliasQuery = await invoke("create_block", { after: aliasMetric, source: ALIAS_QUERY });
      await refresh();

      await executeViaRealKeypress(aliasBlock);
      await executeViaRealKeypress(aliasDeclsRest);
      await executeViaRealKeypress(aliasMetric);
      await executeViaRealKeypress(aliasQuery);

      // ---- Criterion: an alias declared in one block, used in a LATER
      // block, produces a real result through a real keypress chain --
      // not just that parsing succeeds internally. ----
      const afterFirstRun = (await listBlocks()).find((b) => b.id === aliasQuery);
      record("alias_declared_in_one_block_and_used_in_a_later_block_produces_a_real_result", afterFirstRun.output.kind === "Query" && !!afterFirstRun.output.latex, {
        output: afterFirstRun.output,
      });

      // Golden check, same reasoning as the CLI's own byte-identical
      // fixture test (reissner_nordstrom_alias.od vs
      // reissner_nordstrom.od): the SAME metric, `f(r)` written out by
      // hand instead of via the alias, run through the SAME query, must
      // produce the identical LaTeX string -- proof the expansion is
      // faithful end to end, through the real UI, not just that
      // *a* result appeared.
      const directMetricBlock = await invoke("create_block", { after: aliasQuery, source: ALIAS_METRIC.replace(/falias/g, "(1 - 2*M/r)").replace("metric aliasG", "metric aliasGDirect") });
      const directQueryBlock = await invoke("create_block", { after: directMetricBlock, source: "kretschmann aliasGDirect" });
      await refresh();
      await executeViaRealKeypress(directMetricBlock);
      await executeViaRealKeypress(directQueryBlock);
      const directResult = (await listBlocks()).find((b) => b.id === directQueryBlock);
      record("alias_expansion_matches_the_same_expression_written_out_by_hand_byte_for_byte", directResult.output.latex === afterFirstRun.output.latex, {
        withAlias: afterFirstRun.output.latex,
        writtenByHand: directResult.output.latex,
      });

      // ---- Criterion: editing the alias block marks the block that
      // uses it obsolete -- real DOM check (amber class + label), same
      // assertions Phase 4's own obsolescence criteria already use. ----
      await editViaRealEditor(aliasBlock, "falias := 1 - 3*M/r" + "\n");
      const afterAliasEdit = await listBlocks();
      const flags = (bs, id) => bs.find((b) => b.id === id).obsolete;
      record("editing_the_alias_block_marks_the_metric_block_that_uses_it_obsolete", flags(afterAliasEdit, aliasMetric), {
        obsolete: flags(afterAliasEdit, aliasMetric),
      });
      record("editing_the_alias_block_marks_the_query_block_that_depends_on_it_obsolete", flags(afterAliasEdit, aliasQuery), {
        obsolete: flags(afterAliasEdit, aliasQuery),
      });
      const queryDiv = blockDiv(aliasQuery);
      record("obsolete_query_after_alias_edit_shows_the_amber_class_and_label_in_dom", queryDiv.classList.contains("obsolete") && !!queryDiv.querySelector(".obsolete-label"), {
        classList: Array.from(queryDiv.classList),
      });

      // Re-executing the metric (which reconstructs the alias along
      // with it) and then the query clears the marks, using the NEW
      // alias value -- confirms this is a live comparison, not a sticky
      // flag, same as every other obsolescence criterion in this file.
      await executeViaRealKeypress(aliasMetric);
      await executeViaRealKeypress(aliasQuery);
      const afterReexec = (await listBlocks()).find((b) => b.id === aliasQuery);
      record("reexecuting_after_an_alias_change_clears_the_obsolete_mark_and_uses_the_new_value", !afterReexec.obsolete && afterReexec.output.kind === "Query" && !!afterReexec.output.latex, {
        obsolete: afterReexec.obsolete,
        output: afterReexec.output,
      });
    }

    // ---- Phase 8: tensor-result typography (named indices, orbit note
    // subordinated, no prose through KaTeX) + click-to-copy ----
    // Own declaration/chart names (TypoM/TypoTM/typoChart), same
    // isolation reasoning as every phase above. Reissner-Nordstrom
    // again (same metric the app seeds on startup), but through
    // `riemann` specifically -- unlike `kretschmann`/`scalar`, it's a
    // *list* of independent components, the actual shape Problem 1 was
    // about (a bare scalar has no orbit note and no "N components by
    // symmetry"/"M components identically zero" lines to mis-render).
    {
      const TYPO_DECLS =
        "manifold TypoM dim 4\nbundle TypoTM on TypoM dim 4\nchart typoChart on TypoM coords (t, r, theta, phi)\n" +
        "metric typoG on typoChart bundle TypoTM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const typoDecls = await invoke("create_block", { after: null, source: TYPO_DECLS });
      const typoRiemann = await invoke("create_block", { after: typoDecls, source: "riemann typoG" });
      await refresh();
      await executeViaRealKeypress(typoDecls);
      await executeViaRealKeypress(typoRiemann);

      const riemannBlock = (await listBlocks()).find((b) => b.id === typoRiemann);
      const components = (riemannBlock.output && riemannBlock.output.components) || [];
      const summary = (riemannBlock.output && riemannBlock.output.summary) || [];

      // ---- Criterion: indices are chart coordinate names, comma-free,
      // book-style subscript -- never raw integers like "R_{0,1,0,1}".
      // Riemann from a METRIC is fully covariant (the first index was
      // already lowered), so every one of these must be a plain
      // subscript group with no leading "^" at all -- displaying one
      // would misrepresent what was actually computed (this round's
      // own "não subir índice" decision). ----
      // Space-separated inside the braces (`t r \theta`, not `tr\theta`)
      // -- deliberate, see oderom_components::render::format_indices's
      // own doc comment: math mode ignores the space for layout, but it
      // is what keeps a Greek macro from swallowing a directly
      // following plain letter into its own control-word name.
      const namedCovariantFormula = components.find((c) => /^R_\{[a-z\\ ]+\}\s=/.test(c.latex));
      record("riemann_from_metric_shows_named_comma_free_covariant_indices", !!namedCovariantFormula, {
        components: components.map((c) => c.latex),
      });
      record("no_component_formula_uses_raw_comma_separated_integer_indices", !components.some((c) => /_\{\d+,\d+/.test(c.latex) || /\[\d+,\d+/.test(c.latex)), {
        components: components.map((c) => c.latex),
      });
      // Only the INDEX part (before " = ") is checked for a "^" --
      // the VALUE part legitimately has one for every squared term
      // (`r^{4}`, `Q^{2}`, ...), which is exponentiation, not an upper
      // tensor index, and must not be confused with one.
      record("no_component_index_notation_contains_an_upper_index_caret_riemann_from_metric_is_fully_covariant", !components.some((c) => c.latex.split(" = ")[0].includes("^")), {
        indexParts: components.map((c) => c.latex.split(" = ")[0]),
      });

      // ---- Criterion: the orbit-size annotation is never glued into
      // the formula string, and the summary lines are separate,
      // ordinary text -- both are the direct fix for the reported bug
      // (an English parenthetical fed through KaTeX one line at a time
      // with the real formula, coming out italic and unspaced,
      // "4componentsbysymmetry"). ----
      const withOrbitNote = components.find((c) => !!c.orbit_note);
      record("a_component_with_symmetry_partners_carries_a_separate_plain_text_orbit_note", !!withOrbitNote && /^\d+ components by symmetry$/.test(withOrbitNote.orbit_note), {
        orbit_note: withOrbitNote && withOrbitNote.orbit_note,
      });
      record("no_component_formula_string_contains_the_orbit_note_text", !components.some((c) => c.latex.includes("components by symmetry")), {
        components: components.map((c) => c.latex),
      });
      record("summary_lines_are_separate_from_components_and_still_populated", summary.length > 0 && summary.every((line) => !line.includes("\\")), {
        summary,
      });

      // ---- Criterion: the DOM actually reflects this split -- one
      // `.component-row` per shown component, each with its own real
      // KaTeX output, and no KaTeX-rendered node's own text contains
      // the mangled "componentsbysymmetry" run this bug used to
      // produce. ----
      await waitFor(() => blockDiv(typoRiemann).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rows = Array.from(blockDiv(typoRiemann).querySelectorAll(".component-row"));
      record("dom_shows_one_component_row_per_shown_component_each_with_real_katex_output", rows.length === components.length && rows.every((r) => !!r.querySelector(".katex")), {
        rowCount: rows.length,
        componentCount: components.length,
      });
      record("no_katex_rendered_node_in_the_dom_contains_the_mangled_prose_this_bug_used_to_produce", !rows.some((r) => r.querySelector(".katex").textContent.includes("componentsbysymmetry")), {
        katexTexts: rows.map((r) => r.querySelector(".katex").textContent),
      });
      const noteRow = rows.find((r) => r.querySelector(".component-note"));
      record("a_row_with_symmetry_partners_shows_its_note_as_a_separate_dom_element_beside_the_formula_not_inside_it", !!noteRow, {
        noteText: noteRow && noteRow.querySelector(".component-note").textContent,
      });

      // ---- Criterion (Problem 2): clicking a component copies exactly
      // that component's clean LaTeX to the REAL OS clipboard -- read
      // back via `read_clipboard_for_test` (Rust, `arboard`), not just
      // that some click handler fired. Never the orbit note, never any
      // other component. ----
      const targetRow = noteRow || rows[0];
      const targetIndex = rows.indexOf(targetRow);
      const expectedLatex = components[targetIndex].latex;
      for (const type of ["mousedown", "mouseup", "click"]) {
        targetRow.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: window }));
      }
      await waitFor(async () => {
        try {
          return (await invoke("read_clipboard_for_test")) === expectedLatex || undefined;
        } catch (e) {
          return undefined;
        }
      });
      const clipboardText = await invoke("read_clipboard_for_test");
      record("clicking_a_component_copies_exactly_that_components_clean_latex_to_the_real_os_clipboard", clipboardText === expectedLatex, {
        clipboardText,
        expectedLatex,
      });
      record("the_copied_text_never_includes_the_orbit_note_or_any_other_components_formula", !clipboardText.includes("components by symmetry") && components.filter((c, i) => i !== targetIndex).every((c) => clipboardText !== c.latex), {
        clipboardText,
      });

      // ---- Criterion: the click gives non-modal visual feedback (no
      // dialog was opened -- if one had been, the click's own dispatch
      // above would have thrown or hung waiting for it) and a
      // transient "copiado" badge actually appears in the DOM. ----
      const flashAppeared = !!targetRow.querySelector(".copied-flash");
      record("clicking_a_component_shows_a_brief_non_modal_copied_confirmation_in_the_dom", flashAppeared, {
        flashText: flashAppeared && targetRow.querySelector(".copied-flash").textContent,
      });
    }

    // ---- Phase 9: indeterminate functions (f(r)/f'(r), Marco 6 step 4)
    // ----
    // Own declaration/chart names (IndetM/IndetTM/indetChart), same
    // isolation reasoning as every phase above. The acceptance test for
    // this whole feature: a metric with an UNKNOWN f(r) in place of a
    // closed form is accepted (not rejected as "unknown function"), and
    // a real `christoffel` query through the real UI produces symbols
    // containing both `f(r)` and `f'(r)`, legibly, via real KaTeX --
    // never that any equation gets solved (out of scope this round, not
    // checked here on purpose).
    {
      const INDET_DECLS =
        "manifold IndetM dim 4\nbundle IndetTM on IndetM dim 4\nchart indetChart on IndetM coords (t, r, theta, phi)\n" +
        "metric indetG on indetChart bundle IndetTM {\n  [t,t] = -f(r),\n  [r,r] = 1/f(r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const indetDecls = await invoke("create_block", { after: null, source: INDET_DECLS });
      const indetChristoffel = await invoke("create_block", { after: indetDecls, source: "christoffel indetG" });
      await refresh();
      await executeViaRealKeypress(indetDecls);
      await executeViaRealKeypress(indetChristoffel);

      const block = (await listBlocks()).find((b) => b.id === indetChristoffel);
      record("a_metric_with_an_unknown_f_of_r_is_accepted_not_rejected_as_unknown_function", block.output.kind === "Query" && !!block.output.latex && !block.output.message, {
        output: block.output,
      });
      const components = (block.output && block.output.components) || [];
      record("christoffel_of_an_unknown_f_of_r_produces_at_least_one_component", components.length > 0, {
        components: components.map((c) => c.latex),
      });
      record("some_christoffel_component_contains_f_of_r_itself", components.some((c) => c.latex.includes("f(r)")), {
        components: components.map((c) => c.latex),
      });
      record("some_christoffel_component_contains_f_prime_of_r", components.some((c) => c.latex.includes("f'(r)")), {
        components: components.map((c) => c.latex),
      });

      // ---- Criterion: the DOM actually renders this via real KaTeX --
      // a legible formula, not raw unrendered LaTeX source text. ----
      await waitFor(() => blockDiv(indetChristoffel).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rows = Array.from(blockDiv(indetChristoffel).querySelectorAll(".component-row"));
      record("dom_shows_real_katex_output_for_the_indeterminate_function_components", rows.length > 0 && rows.every((r) => !!r.querySelector(".katex")), {
        rowCount: rows.length,
      });
    }
    // ---- Phase 10: geodesic equation (Marco 6 step 4, round B) ----
    // Own declaration/chart names (GeoM/GeoTM/geoChart), same isolation
    // reasoning as every phase above. A real `geodesic tau` query
    // through the real UI on a Schwarzschild-shaped metric must produce
    // four components (one equation per coordinate), rendered via real
    // KaTeX, with LaTeX dot notation (`\ddot{t}`, `\dot{r}`, ...) --
    // this round's own central design point (a coordinate's "free" and
    // "function of the affine parameter" encarnações must never be
    // confused) checked here through the real rendering pipeline, not
    // just the Rust unit/CLI tests that already cover it.
    {
      const GEO_DECLS =
        "manifold GeoM dim 4\nbundle GeoTM on GeoM dim 4\nchart geoChart on GeoM coords (t, r, theta, phi)\n" +
        "metric geoG on geoChart bundle GeoTM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const geoDecls = await invoke("create_block", { after: null, source: GEO_DECLS });
      const geoQuery = await invoke("create_block", { after: geoDecls, source: "geodesic geoG tau" });
      await refresh();
      await executeViaRealKeypress(geoDecls);
      await executeViaRealKeypress(geoQuery);

      const block = (await listBlocks()).find((b) => b.id === geoQuery);
      record("geodesic_of_schwarzschild_is_accepted_and_produces_a_result", block.output.kind === "Query" && !!block.output.latex && !block.output.message, {
        output: block.output,
      });
      const components = (block.output && block.output.components) || [];
      record("geodesic_produces_exactly_four_equations_one_per_coordinate", components.length === 4, {
        components: components.map((c) => c.latex),
      });
      record("geodesic_latex_uses_dot_notation_for_the_acceleration_and_velocity_terms", components.some((c) => c.latex.includes("\\ddot{t}")) && components.some((c) => c.latex.includes("\\dot{r}")), {
        components: components.map((c) => c.latex),
      });
      record("geodesic_latex_never_leaks_the_affine_parameter_name_under_dot_notation", !components.some((c) => c.latex.includes("tau")), {
        components: components.map((c) => c.latex),
      });

      // ---- Criterion: the DOM actually renders this via real KaTeX --
      // same shape every other components-list query already proves
      // (Phase 8/9 above), checked here for geodesic's own output too.
      await waitFor(() => blockDiv(geoQuery).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rows = Array.from(blockDiv(geoQuery).querySelectorAll(".component-row"));
      record("dom_shows_one_real_katex_row_per_geodesic_equation", rows.length === components.length && rows.every((r) => !!r.querySelector(".katex")), {
        rowCount: rows.length,
        componentCount: components.length,
      });
    }

    // ---- Phase 11: the geodesic equation solved for the acceleration
    // (Marco 6 step 4, round C) ----
    // Own declaration/chart names (AccelM/AccelTM/accelChart), same
    // isolation reasoning as every phase above. A real `accel tau`
    // query through the real UI on the same Schwarzschild-shaped metric
    // must produce four `coord'' = ...` equations (never the canonical
    // `= 0` shape `geodesic` itself produces), rendered via real KaTeX,
    // with LaTeX dot notation on BOTH sides of the `=`.
    {
      const ACCEL_DECLS =
        "manifold AccelM dim 4\nbundle AccelTM on AccelM dim 4\nchart accelChart on AccelM coords (t, r, theta, phi)\n" +
        "metric accelG on accelChart bundle AccelTM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const accelDecls = await invoke("create_block", { after: null, source: ACCEL_DECLS });
      const accelQuery = await invoke("create_block", { after: accelDecls, source: "accel accelG tau" });
      await refresh();
      await executeViaRealKeypress(accelDecls);
      await executeViaRealKeypress(accelQuery);

      const block = (await listBlocks()).find((b) => b.id === accelQuery);
      record("accel_of_schwarzschild_is_accepted_and_produces_a_result", block.output.kind === "Query" && !!block.output.latex && !block.output.message, {
        output: block.output,
      });
      const components = (block.output && block.output.components) || [];
      record("accel_produces_exactly_four_solved_equations_one_per_coordinate", components.length === 4, {
        components: components.map((c) => c.latex),
      });
      record("accel_latex_shows_dot_notation_on_both_sides_of_the_equals_sign", components.some((c) => c.latex.startsWith("\\ddot{t} = ")) && components.some((c) => c.latex.includes("\\dot{r}")), {
        components: components.map((c) => c.latex),
      });
      record("accel_never_shows_the_canonical_equals_zero_form", !components.some((c) => c.latex.trim().endsWith("= 0")), {
        components: components.map((c) => c.latex),
      });

      // ---- Criterion: the DOM actually renders this via real KaTeX --
      // same shape every other components-list query already proves.
      await waitFor(() => blockDiv(accelQuery).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rows = Array.from(blockDiv(accelQuery).querySelectorAll(".component-row"));
      record("dom_shows_one_real_katex_row_per_solved_accel_equation", rows.length === components.length && rows.every((r) => !!r.querySelector(".katex")), {
        rowCount: rows.length,
        componentCount: components.length,
      });
    }

    // ---- Phase 12: the Einstein tensor (Marco 6 step 5) ----
    // Own declaration/chart names (EinsteinM/EinsteinTM/einsteinChart),
    // same isolation reasoning as every phase above. Two sub-cases
    // through the real UI: Reissner-Nordstrom (not vacuum -- real,
    // nonzero, typeset components) and Schwarzschild (vacuum -- the
    // golden check, all ten independent components identically zero,
    // shown only via the summary line, never as individual component
    // rows).
    {
      const EINSTEIN_RN_DECLS =
        "manifold EinsteinM dim 4\nbundle EinsteinTM on EinsteinM dim 4\nchart einsteinChart on EinsteinM coords (t, r, theta, phi)\n" +
        "metric einsteinG on einsteinChart bundle EinsteinTM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const einsteinDecls = await invoke("create_block", { after: null, source: EINSTEIN_RN_DECLS });
      const einsteinQuery = await invoke("create_block", { after: einsteinDecls, source: "einstein einsteinG" });
      await refresh();
      await executeViaRealKeypress(einsteinDecls);
      await executeViaRealKeypress(einsteinQuery);

      const block = (await listBlocks()).find((b) => b.id === einsteinQuery);
      record("einstein_of_reissner_nordstrom_is_accepted_and_produces_a_result", block.output.kind === "Query" && !!block.output.latex && !block.output.message, {
        output: block.output,
      });
      const components = (block.output && block.output.components) || [];
      record("einstein_of_reissner_nordstrom_produces_real_nonzero_components", components.length > 0, {
        components: components.map((c) => c.latex),
      });
      record("einstein_component_uses_the_real_g_label_and_named_indices", components.some((c) => /^G_\{[a-z\\ ]+\}\s=/.test(c.latex)), {
        components: components.map((c) => c.latex),
      });

      await waitFor(() => blockDiv(einsteinQuery).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rows = Array.from(blockDiv(einsteinQuery).querySelectorAll(".component-row"));
      record("dom_shows_one_real_katex_row_per_einstein_component", rows.length === components.length && rows.every((r) => !!r.querySelector(".katex")), {
        rowCount: rows.length,
        componentCount: components.length,
      });

      // ---- The golden check, through the real UI: Schwarzschild is
      // vacuum, so einstein must show all ten independent components
      // identically zero -- no component rows at all, only the summary
      // line, same shape `ricci` already has on the same metric. ----
      const VAC_DECLS =
        "manifold EinsteinVacM dim 4\nbundle EinsteinVacTM on EinsteinVacM dim 4\nchart einsteinVacChart on EinsteinVacM coords (t, r, theta, phi)\n" +
        "metric einsteinVacG on einsteinVacChart bundle EinsteinVacTM {\n  [t,t] = -(1 - 2*M/r),\n  [r,r] = 1/(1 - 2*M/r),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const vacDecls = await invoke("create_block", { after: null, source: VAC_DECLS });
      const vacQuery = await invoke("create_block", { after: vacDecls, source: "einstein einsteinVacG" });
      await refresh();
      await executeViaRealKeypress(vacDecls);
      await executeViaRealKeypress(vacQuery);

      const vacBlock = (await listBlocks()).find((b) => b.id === vacQuery);
      const vacComponents = (vacBlock.output && vacBlock.output.components) || [];
      const vacSummary = (vacBlock.output && vacBlock.output.summary) || [];
      record("einstein_of_schwarzschild_shows_no_component_rows_all_ten_are_zero", vacComponents.length === 0, {
        components: vacComponents.map((c) => c.latex),
      });
      record("einstein_of_schwarzschild_summary_names_ten_identically_zero_components", vacSummary.some((line) => line.includes("10") && line.includes("identically zero")), {
        summary: vacSummary,
      });
    }

    // ---- Phase 13: the Weyl tensor and a curvature scalar invariant
    // (Marco 6 step 6) ----
    // Own declaration/chart names (WeylM/WeylTM/weylChart), same
    // isolation reasoning as every phase above. Reissner-Nordstrom (not
    // vacuum) through the real UI: `weyl` must show real, nonzero,
    // typeset components with the real `C` label, and `riccisquare`
    // must show a real nonzero scalar -- both through genuine dispatched
    // keystrokes and real KaTeX, not just the Rust/CLI levels already
    // covered by `oderom-components`'s own test suite.
    {
      const WEYL_DECLS =
        "manifold WeylM dim 4\nbundle WeylTM on WeylM dim 4\nchart weylChart on WeylM coords (t, r, theta, phi)\n" +
        "metric weylG on weylChart bundle WeylTM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}";
      const weylDecls = await invoke("create_block", { after: null, source: WEYL_DECLS });
      const weylQuery = await invoke("create_block", { after: weylDecls, source: "weyl weylG" });
      const riccisquareQuery = await invoke("create_block", { after: weylQuery, source: "riccisquare weylG" });
      await refresh();
      await executeViaRealKeypress(weylDecls);
      await executeViaRealKeypress(weylQuery);
      await executeViaRealKeypress(riccisquareQuery);

      const weylBlock = (await listBlocks()).find((b) => b.id === weylQuery);
      record("weyl_of_reissner_nordstrom_is_accepted_and_produces_a_result", weylBlock.output.kind === "Query" && !!weylBlock.output.latex && !weylBlock.output.message, {
        output: weylBlock.output,
      });
      const weylComponents = (weylBlock.output && weylBlock.output.components) || [];
      record("weyl_of_reissner_nordstrom_produces_real_nonzero_components", weylComponents.length > 0, {
        components: weylComponents.map((c) => c.latex),
      });
      record("weyl_component_uses_the_real_c_label_and_named_indices", weylComponents.some((c) => /^C_\{[a-z\\ ]+\}\s=/.test(c.latex)), {
        components: weylComponents.map((c) => c.latex),
      });

      await waitFor(() => blockDiv(weylQuery).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const weylRows = Array.from(blockDiv(weylQuery).querySelectorAll(".component-row"));
      record("dom_shows_one_real_katex_row_per_weyl_component", weylRows.length === weylComponents.length && weylRows.every((r) => !!r.querySelector(".katex")), {
        rowCount: weylRows.length,
        componentCount: weylComponents.length,
      });

      const rsqBlock = (await listBlocks()).find((b) => b.id === riccisquareQuery);
      record("riccisquare_of_reissner_nordstrom_is_accepted_and_produces_a_result", rsqBlock.output.kind === "Query" && !!rsqBlock.output.latex && !rsqBlock.output.message, {
        output: rsqBlock.output,
      });
      await waitFor(() => blockDiv(riccisquareQuery).querySelectorAll(".component-row .katex").length > 0 || undefined);
      const rsqKatex = blockDiv(riccisquareQuery).querySelector(".component-row .katex");
      record("dom_shows_real_katex_output_for_riccisquare", !!rsqKatex, {
        text: rsqKatex && rsqKatex.textContent,
      });
    }

    // ---- Phase 14: the spacetime gallery, `load` (Rodada Galeria) ----
    // Unlike every phase above, the gallery's own declaration text is
    // NOT under this test's control -- it is `oderom_notebook::gallery`'s
    // fixed, canonical text (the whole point of `load` is pasting that
    // exact text, unprefixed, the same names a hand-typed example would
    // use), so it cannot be given phase-scoped unique names the way
    // every earlier phase's own setup was. That canonical text names
    // its manifold/bundle/metric `M`/`TM`/`g` -- exactly what
    // `seed_example()` (`src-tauri/src/lib.rs`) already named the
    // window's very first, Phase-1 declaration when it opened, and this
    // is the last phase in the file, with nothing later depending on
    // that seed by name -- so it is deleted here first, by matching its
    // exact known source text, purely to give this phase the same
    // collision-free ground every earlier phase got for free from its
    // own unique names. A real click on #gallery-btn,
    // then a real click on the FRW row -- never `invoke("load_gallery",
    // ...)` called directly, which would only prove the Tauri command
    // works, not that the button/panel wiring in `dist/notebook.js`
    // does. Checks the panel actually shows real entries, that picking
    // one pastes the *exact* declaration text `oderom_notebook::gallery`
    // carries (not just "some blocks appeared"), and that the loaded
    // metric computes for real afterward -- through genuine dispatched
    // keystrokes and real KaTeX, exercising a(t) (an indeterminate
    // function directly inside a metric component, this round's own
    // riskiest Group 1 case) live in the browser, not just at the
    // Rust/CLI levels `oderom-notebook/tests/gallery.rs` already covers.
    {
      const SEED_SOURCES = new Set([
        "manifold M dim 4\nbundle TM on M dim 4",
        "chart schw on M coords (t, r, theta, phi)",
        "metric g on schw bundle TM {\n  [t,t] = -(1 - 2*M/r + Q^2/r^2),\n  [r,r] = 1/(1 - 2*M/r + Q^2/r^2),\n  [theta,theta] = r^2,\n  [phi,phi] = r^2 * sin(theta)^2\n}",
      ]);
      for (const b of await listBlocks()) {
        if (SEED_SOURCES.has(b.source)) await invoke("delete_block", { id: b.id });
      }
      await refresh();

      const blocksBeforeGallery = (await listBlocks()).length;

      document.getElementById("gallery-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(() => (document.getElementById("gallery-panel").hidden === false ? true : undefined));
      const rows = Array.from(document.querySelectorAll("#gallery-list .gallery-entry"));
      record("gallery_panel_lists_real_entries", rows.length >= 5, { count: rows.length, names: rows.map((r) => r.dataset.galleryName) });

      const frwRow = rows.find((r) => r.dataset.galleryName === "frw");
      record("gallery_panel_has_an_frw_row", !!frwRow, { names: rows.map((r) => r.dataset.galleryName) });
      frwRow.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));

      await waitFor(() => (document.getElementById("gallery-panel").hidden === true ? true : undefined));
      record("gallery_panel_closes_after_picking_an_entry", document.getElementById("gallery-panel").hidden === true, {});

      await waitFor(async () => ((await listBlocks()).length > blocksBeforeGallery ? true : undefined));
      const afterGallery = await listBlocks();
      const newBlocks = afterGallery.slice(blocksBeforeGallery);
      record("load_pastes_exactly_the_gallery_entrys_own_declaration_blocks", newBlocks.length === 3 && newBlocks[2].source.includes("a(t)^2"), {
        sources: newBlocks.map((b) => b.source),
      });
      record("loaded_blocks_start_never_run_editable_text_not_opaque", newBlocks.every((b) => b.output.kind === "NeverRun"), {
        outputs: newBlocks.map((b) => b.output.kind),
      });

      for (const b of newBlocks) {
        await executeViaRealKeypress(b.id);
      }
      // Explicit target "g" -- by this point in the file many other
      // phases' own metrics coexist in the same live notebook (their own
      // uniquely-prefixed names), so a bare, untargeted "scalar" is
      // itself genuinely ambiguous; "g" is the gallery's own metric name
      // for the FRW entry (see `oderom-notebook/src/gallery.rs`), unique
      // now that the seed's own same-named metric was deleted above.
      const scalarQuery = await invoke("create_block", { after: newBlocks[newBlocks.length - 1].id, source: "scalar g" });
      await refresh();
      await executeViaRealKeypress(scalarQuery);
      const scalarBlock = (await listBlocks()).find((b) => b.id === scalarQuery);
      record("loaded_frw_metric_computes_a_real_scalar_showing_the_scale_factor", scalarBlock.output.kind === "Query" && !!scalarBlock.output.latex && scalarBlock.output.latex.includes("a("), {
        output: scalarBlock.output,
      });
    }

    // ---- Phase 15: "Limpar execução" / "Novo caderno" (reset controls) ----
    // Real clicks on the two new toolbar buttons -- never
    // `invoke("clear_execution")`/`invoke("new_notebook")` directly,
    // which would only prove the Tauri commands work
    // (`oderom-notebook`'s own extensive Rust suite already does that
    // rigorously), not that the button/confirmation-panel wiring in
    // dist/notebook.js actually reaches them. Deliberately the LAST
    // phase in this file: "Novo caderno", once genuinely confirmed,
    // wipes the entire notebook -- every block every earlier phase
    // created -- down to one blank placeholder, so nothing after it
    // could run against a surviving block anyway.
    {
      const clearTestSource = "manifold ClearExecTest dim 2\nbundle ClearExecTM on ClearExecTest dim 2";
      const clearTestId = await invoke("create_block", { after: undefined, source: clearTestSource });
      await refresh();
      await executeViaRealKeypress(clearTestId);
      const beforeClear = (await listBlocks()).find((b) => b.id === clearTestId);
      record("clear_execution_setup_block_has_a_real_execution_count_before_clearing", beforeClear.execution_count != null, { execution_count: beforeClear.execution_count });

      document.getElementById("clear-execution-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(async () => {
        const blocks = await listBlocks();
        return blocks.length > 0 && blocks.every((b) => b.execution_count == null) ? true : undefined;
      });
      const afterClear = await listBlocks();
      const clearedTestBlock = afterClear.find((b) => b.source === clearTestSource);
      record("clear_execution_button_resets_every_gutter_to_empty_and_keeps_source_text", afterClear.every((b) => b.execution_count == null) && !!clearedTestBlock, {
        execution_counts: afterClear.map((b) => b.execution_count),
        sourcePreserved: !!clearedTestBlock,
      });
      record("clear_execution_resets_every_output_to_never_run", afterClear.every((b) => b.output.kind === "NeverRun"), { kinds: afterClear.map((b) => b.output.kind) });

      // The button-driven reset must be exactly as complete as the
      // Rust-level guarantee it backs: re-running the same (untouched)
      // declaration text after a click on "Limpar execução" starts the
      // gutter numbering over at [1], not a continuation of whatever
      // number was already in flight before the click.
      await executeViaRealKeypress(clearedTestBlock.id);
      const reExecuted = (await listBlocks()).find((b) => b.id === clearedTestBlock.id);
      const gutterText = blockDiv(clearedTestBlock.id).querySelector(".block-gutter").textContent;
      record("clear_execution_reexecuting_shows_bracket_one_not_continued_numbering", reExecuted.execution_count === 1 && gutterText === "[1]", {
        execution_count: reExecuted.execution_count,
        gutterText,
      });

      // "Novo caderno": open, cancel (via its own button), confirm
      // nothing changed either time -- THEN open again and genuinely
      // confirm via Escape once more, and only then via the real
      // "Apagar tudo" click.
      const blocksBeforeNewNotebook = await listBlocks();

      document.getElementById("new-notebook-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(() => (document.getElementById("new-notebook-confirm-panel").hidden === false ? true : undefined));
      record("new_notebook_confirm_panel_opens_on_click_without_destroying_anything", document.getElementById("new-notebook-confirm-panel").hidden === false, {});
      const stillSameAfterOpen = await listBlocks();
      record("opening_the_new_notebook_confirmation_does_not_touch_any_block", stillSameAfterOpen.length === blocksBeforeNewNotebook.length, {
        before: blocksBeforeNewNotebook.length,
        after: stillSameAfterOpen.length,
      });

      document.getElementById("new-notebook-cancel-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(() => (document.getElementById("new-notebook-confirm-panel").hidden === true ? true : undefined));
      const afterCancel = await listBlocks();
      record("new_notebook_cancel_closes_the_panel_and_leaves_every_block_untouched", afterCancel.length === blocksBeforeNewNotebook.length, {
        blocksBefore: blocksBeforeNewNotebook.length,
        blocksAfter: afterCancel.length,
      });

      // Escape must behave exactly like Cancelar -- close only, never confirm.
      document.getElementById("new-notebook-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(() => (document.getElementById("new-notebook-confirm-panel").hidden === false ? true : undefined));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
      await waitFor(() => (document.getElementById("new-notebook-confirm-panel").hidden === true ? true : undefined));
      const afterEscape = await listBlocks();
      record("escape_on_the_new_notebook_confirmation_also_only_cancels", afterEscape.length === blocksBeforeNewNotebook.length, { blocksAfter: afterEscape.length });

      // Now genuinely confirm: real click on the red "Apagar tudo" button.
      document.getElementById("new-notebook-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(() => (document.getElementById("new-notebook-confirm-panel").hidden === false ? true : undefined));
      document.getElementById("new-notebook-confirm-btn").dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, view: window }));
      await waitFor(async () => {
        const blocks = await listBlocks();
        return blocks.length === 1 && blocks[0].source === "" ? true : undefined;
      });
      const afterConfirm = await listBlocks();
      record("new_notebook_confirm_produces_a_single_blank_block", afterConfirm.length === 1 && afterConfirm[0].source === "" && afterConfirm[0].output.kind === "NeverRun", {
        blocks: afterConfirm,
      });
      record("new_notebook_confirm_panel_closes_after_confirming", document.getElementById("new-notebook-confirm-panel").hidden === true, {});
    }
  } catch (e) {
    record("uncaught_exception", false, { message: e.message, stack: e.stack });
  }

  await finish();
})();
