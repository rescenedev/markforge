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
    rows_cache: RefCell<HashMap<PathBuf, (u64, Rc<Vec<Row>>)>>,
}

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
        if let Some((cached_gen, rows)) = self.rows_cache.borrow().get(root) {
            if *cached_gen == self.generation {
                return rows.clone();
            }
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
