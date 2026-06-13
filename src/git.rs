//! Minimal git integration via the system `git` CLI.
//!
//! Everything here shells out to `git` and is designed to run on the
//! background executor — never call these on the UI thread. Using the CLI
//! (instead of libgit2) keeps auth working exactly like the user's terminal
//! (ssh agent, credential helpers) at zero build cost.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Worktree state of one file, in display priority order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileState {
    Conflicted,
    Modified,
    Staged,
    Untracked,
}

impl FileState {
    /// One-letter badge for the file tree.
    pub fn badge(self) -> &'static str {
        match self {
            Self::Conflicted => "!",
            Self::Modified => "M",
            Self::Staged => "A",
            Self::Untracked => "U",
        }
    }
}

/// Snapshot of a repository's status, keyed by absolute paths.
#[derive(Clone, Default, PartialEq)]
pub struct RepoStatus {
    /// Repository root; `None` when the folder isn't inside a git repo.
    pub root: Option<PathBuf>,
    /// Current branch name ("HEAD" while detached).
    pub branch: Option<String>,
    /// All local branch names.
    pub branches: Vec<String>,
    pub files: HashMap<PathBuf, FileState>,
    /// Directories containing at least one changed file (for tree dots).
    pub dirty_dirs: HashSet<PathBuf>,
}

impl RepoStatus {
    pub fn state_of(&self, path: &Path) -> Option<FileState> {
        self.files.get(path).copied()
    }

    pub fn dir_is_dirty(&self, path: &Path) -> bool {
        self.dirty_dirs.contains(path)
    }
}

/// Run `git` in `dir` and return stdout on success.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// One entry of a file's commit history.
#[derive(Clone)]
pub struct LogEntry {
    pub hash: String,
    pub date: String,
    pub subject: String,
}

/// Recent commits touching `file` (newest first).
pub fn file_log(dir: &Path, file: &str, limit: usize) -> Result<Vec<LogEntry>, String> {
    let out = git(
        dir,
        &[
            "log",
            &format!("-n{limit}"),
            "--format=%h%x09%ad%x09%s",
            "--date=short",
            "--",
            file,
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            Some(LogEntry {
                hash: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                subject: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect())
}

/// Contents of `rel_path` at `rev` (e.g. "HEAD" or a short hash).
pub fn show_file_at(dir: &Path, rev: &str, rel_path: &str) -> Result<String, String> {
    git(dir, &["show", &format!("{rev}:{rel_path}")])
}

/// Stage everything and commit with `message`. Returns git's summary line.
pub fn commit_all(dir: &Path, message: &str) -> Result<String, String> {
    git(dir, &["add", "-A"])?;
    git(dir, &["commit", "-m", message]).map(|out| {
        out.lines()
            .next()
            .unwrap_or("committed")
            .trim()
            .to_string()
    })
}

/// Status for the first candidate directory that is inside a git repo.
/// Lets a click target take priority while gracefully falling back (e.g. a
/// non-repo folder keeps showing the open document's repository).
pub fn repo_status_first(candidates: &[PathBuf]) -> RepoStatus {
    for dir in candidates {
        let status = repo_status(dir);
        if status.root.is_some() {
            return status;
        }
    }
    RepoStatus::default()
}

/// Full status snapshot for the repository containing `dir`.
pub fn repo_status(dir: &Path) -> RepoStatus {
    let Ok(root) = git(dir, &["rev-parse", "--show-toplevel"]) else {
        return RepoStatus::default();
    };
    let root = PathBuf::from(root.trim());

    let branch = git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let branches: Vec<String> = git(dir, &["branch", "--format=%(refname:short)"])
        .map(|out| {
            out.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut files = HashMap::new();
    let mut dirty_dirs = HashSet::new();
    if let Ok(porcelain) = git(dir, &["status", "--porcelain", "-z"]) {
        let mut tokens = porcelain.split('\0');
        while let Some(entry) = tokens.next() {
            if entry.len() < 4 {
                continue;
            }
            let (xy, rel) = entry.split_at(3);
            let x = xy.as_bytes()[0] as char;
            let y = xy.as_bytes()[1] as char;
            // Renames/copies carry the original path as an extra token.
            if matches!(x, 'R' | 'C') {
                tokens.next();
            }

            let state = if x == 'U' || y == 'U' || (x == 'D' && y == 'D') || (x == 'A' && y == 'A')
            {
                FileState::Conflicted
            } else if xy.starts_with("??") {
                FileState::Untracked
            } else if matches!(y, 'M' | 'D' | 'T') {
                FileState::Modified
            } else {
                FileState::Staged
            };

            let path = root.join(rel.trim_end_matches('/'));
            // Mark every ancestor directory (up to the root) as dirty.
            let mut dir = path.parent();
            while let Some(d) = dir {
                if !d.starts_with(&root) || !dirty_dirs.insert(d.to_path_buf()) {
                    break;
                }
                dir = d.parent();
            }
            files.insert(path, state);
        }
    }

    RepoStatus {
        root: Some(root),
        branch,
        branches,
        files,
        dirty_dirs,
    }
}
