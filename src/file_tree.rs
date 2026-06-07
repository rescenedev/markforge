//! File-explorer data model for the sidebar.
//!
//! Holds the opened root directory, the set of expanded folders, and a cache of
//! directory listings (read lazily on expand). `rows()` flattens the visible
//! tree into a list the view renders.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

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
    root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    cache: HashMap<PathBuf, Vec<FsEntry>>,
}

impl FileTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn is_open(&self) -> bool {
        self.root.is_some()
    }

    /// Open a new root directory (resets expansion + cache).
    pub fn open(&mut self, root: PathBuf) {
        self.expanded.clear();
        self.cache.clear();
        self.ensure(&root);
        self.root = Some(root);
    }

    /// Toggle a directory's expanded state, reading its contents on first open.
    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
            self.ensure(dir);
        }
    }

    /// Re-read every cached directory from disk (manual refresh).
    pub fn refresh(&mut self) {
        let dirs: Vec<PathBuf> = self.cache.keys().cloned().collect();
        self.cache.clear();
        for d in dirs {
            self.ensure(&d);
        }
    }

    /// Collapse all expanded folders.
    pub fn collapse_all(&mut self) {
        self.expanded.clear();
    }

    fn ensure(&mut self, dir: &Path) {
        if !self.cache.contains_key(dir) {
            self.cache.insert(dir.to_path_buf(), read_dir_sorted(dir));
        }
    }

    /// Flatten the visible tree (root's contents + expanded descendants).
    pub fn rows(&self) -> Vec<Row> {
        let mut out = Vec::new();
        if let Some(root) = &self.root {
            self.walk(root, 0, &mut out);
        }
        out
    }

    fn walk(&self, dir: &Path, depth: usize, out: &mut Vec<Row>) {
        let Some(entries) = self.cache.get(dir) else {
            return;
        };
        for e in entries {
            let expanded = e.is_dir && self.expanded.contains(&e.path);
            out.push(Row {
                entry: e.clone(),
                depth,
                expanded,
            });
            if expanded {
                self.walk(&e.path, depth + 1, out);
            }
        }
    }
}

/// Read a directory, directories first, then case-insensitive by name.
fn read_dir_sorted(dir: &Path) -> Vec<FsEntry> {
    let mut entries: Vec<FsEntry> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| {
                let path = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let name = e.file_name().to_string_lossy().to_string();
                FsEntry { path, name, is_dir }
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
