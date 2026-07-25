//! Save/load the notebook as plain text -- DESIGN-NOTEBOOK.md section
//! 7. Blocks separated by a line that is exactly `%%`; only source text
//! is ever written, never output ("nada recalcula sozinho" applies to
//! opening a saved notebook too). Text, not JSON: diffs well in git,
//! which is the explicit reason to version notebooks this way.

use crate::block::Block;
use crate::notebook::Notebook;
use std::io;
use std::path::Path;

const DELIMITER: &str = "%%";

fn is_delimiter_line(line: &str) -> bool {
    line.trim_end_matches('\r') == DELIMITER
}

/// A block's own source happens to contain a line that is exactly the
/// delimiter -- refusing to save is the only safe response (writing it
/// anyway would silently split one block into two on the next load,
/// with no way to tell that happened).
#[derive(Debug)]
pub struct BlockContainsDelimiter {
    /// 1-based, for a human-facing message -- matches how blocks are
    /// already numbered elsewhere (`[1]`, `[2]`, ... in the REPL).
    pub block_number: usize,
}

impl std::fmt::Display for BlockContainsDelimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "block {} contains a line that is exactly `%%`, the notebook file's own block delimiter -- cannot save without corrupting the file",
            self.block_number
        )
    }
}

impl std::error::Error for BlockContainsDelimiter {}

/// Pure text-building, no I/O -- kept separate from [`save`] so it's
/// directly unit-testable and so the delimiter check happens *before*
/// anything is written, never after a partial write.
pub fn render(blocks: &[Block]) -> Result<String, BlockContainsDelimiter> {
    for (i, block) in blocks.iter().enumerate() {
        if block.source.lines().any(is_delimiter_line) {
            return Err(BlockContainsDelimiter { block_number: i + 1 });
        }
    }
    let mut out = String::new();
    for (i, block) in blocks.iter().enumerate() {
        if i > 0 {
            out.push_str(DELIMITER);
            out.push('\n');
        }
        out.push_str(&block.source);
        if !block.source.ends_with('\n') {
            out.push('\n');
        }
    }
    Ok(out)
}

/// Splits `text` back into block source strings on `%%` delimiter
/// lines. An empty (or whitespace-only) file yields zero blocks, not
/// one empty one -- the natural reading of "nothing was ever saved
/// here", distinct from a file that has one genuinely empty block on
/// purpose.
pub fn parse_sources(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if is_delimiter_line(line) {
            blocks.push(std::mem::take(&mut current));
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    blocks.push(current);
    blocks.into_iter().map(|s| s.strip_suffix('\n').map(str::to_string).unwrap_or(s)).collect()
}

/// Writes `notebook`'s blocks to `path` -- refuses (see
/// [`BlockContainsDelimiter`]) rather than risk corrupting the file.
/// Remembers `path` as the notebook's `current_path` on success, same
/// as `load` does -- purely informational (what a bare "save" would
/// reuse, what to show as the notebook's name), never consulted by any
/// of `Notebook`'s own logic.
pub fn save(notebook: &mut Notebook, path: &Path) -> io::Result<()> {
    let text = render(notebook.blocks()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    std::fs::write(path, &text)?;
    notebook.set_current_path(Some(path.to_path_buf()));
    Ok(())
}

/// Reads `path` into a fresh [`Notebook`] -- every block starts
/// `NeverRun`; nothing is executed and no `Session` state exists yet
/// until the caller runs a block (DESIGN-NOTEBOOK.md section 7: opening
/// never executes anything, output is never persisted so there is none
/// to restore).
pub fn load(path: &Path) -> io::Result<Notebook> {
    let text = std::fs::read_to_string(path)?;
    let mut notebook = Notebook::new();
    for source in parse_sources(&text) {
        notebook.create_block_after(None, source);
    }
    notebook.set_current_path(Some(path.to_path_buf()));
    Ok(notebook)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockOutput;

    #[test]
    fn render_then_parse_sources_round_trips_block_boundaries() {
        let mut nb = Notebook::new();
        nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.create_block_after(None, "chart schw on M coords (t, r)".to_string());
        nb.create_block_after(None, "kretschmann".to_string());

        let text = render(nb.blocks()).unwrap();
        let sources = parse_sources(&text);
        assert_eq!(sources, vec!["manifold M dim 4", "chart schw on M coords (t, r)", "kretschmann"]);
    }

    #[test]
    fn a_block_containing_the_literal_delimiter_line_refuses_to_render() {
        let mut nb = Notebook::new();
        nb.create_block_after(None, "manifold M dim 4".to_string());
        nb.create_block_after(None, "some text\n%%\nmore text".to_string());
        let err = render(nb.blocks()).unwrap_err();
        assert_eq!(err.block_number, 2);
    }

    #[test]
    fn an_empty_file_loads_as_zero_blocks() {
        assert_eq!(parse_sources(""), Vec::<String>::new());
        assert_eq!(parse_sources("   \n  \n"), Vec::<String>::new());
    }

    #[test]
    fn a_single_block_with_no_delimiter_round_trips() {
        let mut nb = Notebook::new();
        nb.create_block_after(None, "kretschmann".to_string());
        let text = render(nb.blocks()).unwrap();
        assert_eq!(text, "kretschmann\n");
        assert_eq!(parse_sources(&text), vec!["kretschmann"]);
    }

    #[test]
    fn save_then_load_round_trips_source_and_starts_every_block_fresh() {
        let mut nb = Notebook::new();
        let id = nb.create_block_after(None, "manifold M dim 4\nbundle TM on M dim 4".to_string());
        nb.create_block_after(None, "kretschmann".to_string());
        nb.execute_block(id); // give it a non-NeverRun output, to prove it's NOT persisted

        let path = tempfile_path("notebook_round_trip");
        save(&mut nb, &path).unwrap();
        let loaded = load(&path).unwrap();
        std::fs::remove_file(&path).ok();

        let sources: Vec<&str> = loaded.blocks().iter().map(|b| b.source.as_str()).collect();
        assert_eq!(sources, vec!["manifold M dim 4\nbundle TM on M dim 4", "kretschmann"]);
        assert!(loaded.blocks().iter().all(|b| matches!(b.output, BlockOutput::NeverRun)), "loading must never restore any output");
        assert!(loaded.session().document().is_none(), "loading must never execute anything");
    }

    #[test]
    fn save_and_load_both_remember_the_path_as_current_path() {
        let mut nb = Notebook::new();
        assert_eq!(nb.current_path(), None, "a freshly created notebook has no current_path yet");
        nb.create_block_after(None, "kretschmann".to_string());

        let path = tempfile_path("notebook_current_path");
        save(&mut nb, &path).unwrap();
        assert_eq!(nb.current_path(), Some(path.as_path()));

        let loaded = load(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(loaded.current_path(), Some(path.as_path()));
    }

    #[test]
    fn saving_a_notebook_with_a_poisoned_block_leaves_no_file_behind() {
        let mut nb = Notebook::new();
        nb.create_block_after(None, "line one\n%%\nline two".to_string());
        let path = tempfile_path("notebook_refuses_to_save");
        std::fs::remove_file(&path).ok();
        let err = save(&mut nb, &path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(!path.exists(), "a refused save must not leave a partial/corrupt file on disk");
        assert_eq!(nb.current_path(), None, "a refused save must not update current_path either");
    }

    fn tempfile_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("oderom-notebook-test-{name}-{}-{}", std::process::id(), unique()))
    }

    fn unique() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64;
        t.wrapping_add(n)
    }
}
