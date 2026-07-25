# Vendored frontend assets

Committed as files, not fetched from a CDN and not installed via npm --
`oderom-app` has no Node/npm dependency at all (DESIGN-NOTEBOOK.md,
Etapa 3a-2: "sem Node"), and the app has to open with no network access,
indefinitely, not just today.

## KaTeX

- Version: **0.18.1**
- Source: `https://registry.npmjs.org/katex/-/katex-0.18.1.tgz` (npm's
  own package tarball, fetched directly over HTTPS -- not the npm CLI,
  no `node_modules`)
- License: MIT (`katex/LICENSE`, copied from the package)
- Files taken from the package's own `dist/`: `katex.min.js`,
  `katex.min.css`, `fonts/*` -- nothing else (no contrib scripts, no
  source maps, no `dist/katex.js` unminified variant).

## CodeMirror 5

- Version: **5.65.21** (the last CodeMirror **5** line -- the
  `codemirror` npm package's `latest` tag now points at CodeMirror 6,
  which requires a bundler; deliberately not used here, see
  DESIGN-NOTEBOOK.md)
- Source: `https://registry.npmjs.org/codemirror/-/codemirror-5.65.21.tgz`
- License: MIT (`codemirror/LICENSE`, copied from the package)
- Files taken: `lib/codemirror.js`, `lib/codemirror.css` (the core),
  plus `addon/mode/simple.js` -> `addon-mode-simple.js` (the one addon
  needed for `CodeMirror.defineSimpleMode`, used to declare ODEROM's
  own highlighting rules below). No bundled language mode: ODEROM's
  `.od`/query grammar isn't one of CodeMirror 5's built-ins, so
  `notebook.js` registers a small, hand-written mode via
  `defineSimpleMode` (highlights the ten declaration/query keywords,
  numbers, and `#` comments -- a lexical approximation of
  `oderom_cli::parser`'s real grammar for display only; the actual
  parsing/validation always happens in Rust via the Tauri commands,
  never in this mode).

## Updating

Bump the version number in the tarball URL above, redownload, and
replace these files in place -- same license, same "just the built
files" scope. Update the two version numbers in this file when you do.
