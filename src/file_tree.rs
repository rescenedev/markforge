//! File-explorer data model for the sidebar.
//!
//! Tracks which folders are expanded and caches directory listings (read
//! lazily on expand). The sidebar is a multi-root tree: any folder (a PLACES
//! entry, a favorite, an iCloud-notes folder, an opened folder) can be
//! expanded in place. [`rows_under`] flattens one root's visible subtree.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::import::is_supported_doc;

/// A single filesystem entry (file or directory).
#[derive(Clone)]
pub struct FsEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
}

/// A flattened, indented row for rendering.
#[derive(Clone)]
pub struct Row {
    pub entry: FsEntry,
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Default)]
pub struct FileTree {
    expanded: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<FsEntry>>,
    /// Bumped on every mutation to invalidate the per-root row caches.
    generation: u64,
    /// One flattened subtree per root, cached until `generation` changes —
    /// `rows_under` runs every frame, so re-walking large folders janks.
    rows_cache: RefCell<RowCache>,
}

/// Per-root flattened rows tagged with the `generation` they were built at.
type RowCache = HashMap<PathBuf, (u64, Rc<Vec<Row>>)>;

impl FileTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle a directory's expanded state, reading its contents on first
    /// open. Returns `true` when the directory is now expanded.
    pub fn toggle(&mut self, dir: &Path) -> bool {
        self.generation = self.generation.wrapping_add(1);
        if self.expanded.remove(dir) {
            false
        } else {
            self.expanded.insert(dir.to_path_buf());
            self.ensure(dir);
            true
        }
    }

    /// Expand a directory (no-op if already expanded).
    pub fn expand(&mut self, dir: &Path) {
        if !self.expanded.contains(dir) {
            self.expanded.insert(dir.to_path_buf());
            self.ensure(dir);
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Re-read every expanded directory from disk (manual refresh).
    pub fn refresh(&mut self) {
        self.cache.clear();
        let dirs: Vec<PathBuf> = self.expanded.iter().cloned().collect();
        for d in dirs {
            self.ensure(&d);
        }
        self.generation = self.generation.wrapping_add(1);
    }

    /// Collapse every folder and drop cached listings.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.cache.clear();
        self.generation = self.generation.wrapping_add(1);
    }

    fn ensure(&mut self, dir: &Path) {
        if !self.cache.contains_key(dir) {
            self.cache.insert(dir.to_path_buf(), read_dir_sorted(dir));
        }
    }

    /// Rows for `root` as an expandable subtree: the root itself at depth 0,
    /// then its expanded descendants. Cached per generation.
    pub fn rows_under(&self, root: &Path) -> Rc<Vec<Row>> {
        if let Some((cached_gen, rows)) = self.rows_cache.borrow().get(root)
            && *cached_gen == self.generation {
                return rows.clone();
            }
        let mut out = Vec::new();
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| root.to_string_lossy().to_string());
        let expanded = self.expanded.contains(root);
        out.push(Row {
            entry: FsEntry { path: root.to_path_buf(), name, is_dir: true },
            depth: 0,
            expanded,
        });
        if expanded {
            self.walk(root, 1, &mut out);
        }
        let rows = Rc::new(out);
        self.rows_cache
            .borrow_mut()
            .insert(root.to_path_buf(), (self.generation, rows.clone()));
        rows
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let Some(entries) = self.cache.get(dir) else {
            return;
        };
        for e in entries {
            let expanded = e.is_dir && self.expanded.contains(&e.path);
            out.push(Row { entry: e.clone(), depth, expanded });
            if expanded {
                self.walk(&e.path, depth + 1, out);
            }
        }
    }
}

/// Read a directory, hiding dotfiles and files MarkForge can't open,
/// directories first, then case-insensitive by name.
fn read_dir_sorted(dir: &Path) -> Vec<FsEntry> {
    let mut entries: Vec<FsEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    return None; // .git, .DS_Store, …
                }
                let path = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if !is_dir && !is_supported_doc(&path) {
                    return None; // binaries etc. would only open as an error
                }
                Some(FsEntry { path, name, is_dir })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A unique scratch directory (no `tempfile` dependency).
    fn temp_dir(label: &str) -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "markforge-ft-{label}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn read_dir_sorted_filters_and_orders() {
        let dir = temp_dir("readdir");
        std::fs::create_dir(dir.join("zeta")).unwrap();
        std::fs::create_dir(dir.join("alpha")).unwrap();
        for f in ["b.md", "A.md", "c.txt", ".hidden", "binary.exe"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let names: Vec<String> = read_dir_sorted(&dir).into_iter().map(|e| e.name).collect();
        // dotfiles + unsupported files dropped; dirs first, then case-insensitive.
        assert_eq!(names, ["alpha", "zeta", "A.md", "b.md", "c.txt"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_dir_sorted_missing_dir_is_empty() {
        let dir = std::env::temp_dir().join("markforge-ft-definitely-not-here-xyz");
        assert!(read_dir_sorted(&dir).is_empty());
    }

    #[test]
    fn rows_under_expands_lazily() {
        let dir = temp_dir("rows");
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("child.md"), b"x").unwrap();
        std::fs::write(dir.join("top.md"), b"x").unwrap();

        let mut ft = FileTree::new();
        ft.expand(&dir);
        let rows = ft.rows_under(&dir);
        assert_eq!(rows[0].depth, 0, "root is depth 0");
        assert!(rows.iter().any(|r| r.depth == 1 && r.entry.name == "sub"));
        assert!(rows.iter().any(|r| r.depth == 1 && r.entry.name == "top.md"));
        // "sub" isn't expanded yet, so its child stays hidden.
        assert!(!rows.iter().any(|r| r.entry.name == "child.md"));

        ft.expand(&dir.join("sub"));
        let rows = ft.rows_under(&dir);
        assert!(rows.iter().any(|r| r.depth == 2 && r.entry.name == "child.md"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn toggle_reports_and_flips_state() {
        let dir = temp_dir("toggle");
        assert!(ft_toggle_twice(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn ft_toggle_twice(dir: &Path) -> bool {
        let mut ft = FileTree::new();
        let opened = ft.toggle(dir); // now expanded
        let closed = ft.toggle(dir); // now collapsed
        opened && !closed
    }
}
