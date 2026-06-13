//! Import Apple/iCloud Notes into a folder of Markdown files.
//!
//! Uses the Notes app's AppleScript interface (stable across macOS versions,
//! unlike the gzip+protobuf SQLite store). To stay within Apple Event size
//! limits, the script writes a delimited dump straight to a temp file
//! (per-note, so a 600-note folder never returns one giant reply); Rust then
//! streams that file record-by-record, strips inline image data, converts the
//! HTML body to Markdown, and writes one `.md` per note under
//! `dest/<folder>/<title>.md`. Read-only with respect to Notes.

use std::collections::HashSet;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

// Delimiters chosen to be vanishingly unlikely in note HTML.
const FIELD: &[u8] = b"@@MF-F-9a3f@@";
const RECORD: &[u8] = b"@@MF-R-9a3f@@";

/// AppleScript: dump every note (folder, title, body HTML) to `argv[1]`,
/// writing per note so no single Apple Event reply blows the size limit.
const SCRIPT: &str = r#"on run argv
  set dumpPath to item 1 of argv
  set fh to open for access (POSIX file dumpPath) with write permission
  set eof of fh to 0
  tell application "Notes"
    repeat with f in folders
      set fn to name of f
      repeat with n in (notes of f)
        write (fn & "@@MF-F-9a3f@@" & (name of n) & "@@MF-F-9a3f@@" & (body of n) & "@@MF-R-9a3f@@") to fh as «class utf8»
      end repeat
    end repeat
  end tell
  close access fh
  return "ok"
end run"#;

/// Where imported notes are written (and re-opened as a folder).
pub fn import_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/MarkForge")
            .join("iCloud Notes"),
    )
}

/// Export every note to `dest` as Markdown. Returns the number written.
/// Safe to run on the background executor.
pub fn export(dest: &Path) -> io::Result<usize> {
    let dump = std::env::temp_dir().join("markforge-notes-dump.txt");
    run_script(&dump)?;

    let converter = htmd::HtmlToMarkdown::new();
    let mut reader = RecordReader::new(std::fs::File::open(&dump)?);
    let mut used_per_dir: std::collections::HashMap<PathBuf, HashSet<String>> =
        std::collections::HashMap::new();
    let mut written = 0usize;

    while let Some(record) = reader.next_record()? {
        let Some((folder, title, body)) = split3(&record) else {
            continue;
        };
        let dir = dest.join(sanitize(&folder));
        std::fs::create_dir_all(&dir)?;

        let stem = {
            let used = used_per_dir.entry(dir.clone()).or_default();
            unique_stem(used, sanitize_title(&title))
        };
        let html = strip_inline_images(&body);
        let markdown = converter.convert(&html).unwrap_or(html);
        std::fs::write(dir.join(format!("{stem}.md")), markdown)?;
        written += 1;
    }

    let _ = std::fs::remove_file(&dump);
    Ok(written)
}

fn run_script(dump: &Path) -> io::Result<()> {
    let output = Command::new("osascript")
        .arg("-e")
        .arg(SCRIPT)
        .arg(dump)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        Err(io::Error::other(
            if err.contains("-1743") || err.contains("not allow") || err.contains("not authoriz") {
                "Notes access denied — grant Automation permission in System \
                 Settings ▸ Privacy & Security ▸ Automation."
                    .to_string()
            } else {
                format!("Notes export failed: {}", err.trim())
            },
        ))
    }
}

/// Split a record's bytes into (folder, title, body) UTF-8 strings.
fn split3(record: &[u8]) -> Option<(String, String, String)> {
    let i = find(record, FIELD)?;
    let rest = &record[i + FIELD.len()..];
    let j = find(rest, FIELD)?;
    let folder = String::from_utf8_lossy(&record[..i]).into_owned();
    let title = String::from_utf8_lossy(&rest[..j]).into_owned();
    let body = String::from_utf8_lossy(&rest[j + FIELD.len()..]).into_owned();
    Some((folder, title, body))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Replace inline `data:` image payloads with an empty src so the export
/// stays small (the largest notes embed tens of MB of base64 images).
fn strip_inline_images(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find("data:") {
        out.push_str(&rest[..pos]);
        // Drop everything up to the next quote that closes the attribute.
        let after = &rest[pos..];
        match after.find(['"', '\'']) {
            Some(end) => rest = &after[end..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Streaming splitter on the RECORD delimiter (bounds memory to one note).
/// `scanned` tracks how far the buffer has been searched so a huge record
/// (notes can embed tens of MB of base64) isn't re-scanned from the start on
/// every read — that turns the split quadratic.
struct RecordReader<R: Read> {
    reader: R,
    buf: Vec<u8>,
    scanned: usize,
    eof: bool,
}

impl<R: Read> RecordReader<R> {
    fn new(reader: R) -> Self {
        Self { reader, buf: Vec::new(), scanned: 0, eof: false }
    }

    fn next_record(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(rel) = find(&self.buf[self.scanned..], RECORD) {
                let i = self.scanned + rel;
                let record: Vec<u8> = self.buf.drain(..i).collect();
                self.buf.drain(..RECORD.len());
                self.scanned = 0;
                if record.is_empty() {
                    continue;
                }
                return Ok(Some(record));
            }
            if self.eof {
                self.scanned = self.buf.len();
                return Ok(None);
            }
            // Everything before the last (needle-1) bytes can't start a match.
            self.scanned = self.buf.len().saturating_sub(RECORD.len() - 1);
            let mut chunk = [0u8; 256 * 1024];
            let n = self.reader.read(&mut chunk)?;
            if n == 0 {
                self.eof = true;
            } else {
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

/// Sanitize a folder/path component (no separators, no leading dot).
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() { "Untitled".to_string() } else { trimmed.to_string() }
}

/// A filename stem from a note title (first line), capped to a sane length.
fn sanitize_title(title: &str) -> String {
    let first_line = title.lines().next().unwrap_or("").trim();
    sanitize(first_line).chars().take(80).collect::<String>().trim().to_string()
}

/// Ensure the stem is unique within a folder (append -2, -3, … on collision).
fn unique_stem(used: &mut HashSet<String>, base: String) -> String {
    let base = if base.is_empty() { "Untitled".to_string() } else { base };
    if used.insert(base.clone()) {
        return base;
    }
    for n in 2.. {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}
