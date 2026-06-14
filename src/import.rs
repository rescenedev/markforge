//! Read-only document importers: `.docx` / `.hwpx` / `.pdf` → Markdown text.
//!
//! These formats are binary containers, so they open as converted, read-only
//! documents — ⌘S saves a Markdown copy instead of touching the original.
//! All functions here are pure and safe to run on the background executor.

use std::io::{self, Read};
use std::path::Path;

use quick_xml::Reader;
use quick_xml::events::Event;

/// Cap converted output so a huge book can't beachball the preview.
const IMPORT_MAX_BYTES: usize = 2 * 1024 * 1024;

/// Extensions that open through a converter (and are therefore read-only).
pub fn is_imported_doc(path: &Path) -> bool {
    matches!(
        ext_lowercase(path).as_deref(),
        Some("docx" | "hwpx" | "pdf")
    )
}

/// Document extensions MarkForge knows how to display: Markdown and plain
/// text, code files (highlighted), and convertible containers.
pub fn is_supported_doc(path: &Path) -> bool {
    matches!(
        ext_lowercase(path).as_deref(),
        Some(
            // markdown & plain text
            "md" | "markdown" | "mdown" | "mkd" | "text" | "txt"
            // code (must stay in sync with DocKind::for_path)
            | "json" | "jsonc" | "py" | "rs"
            | "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx"
            | "sh" | "bash" | "zsh" | "go" | "html" | "htm"
            | "css" | "yaml" | "yml" | "toml"
            // converted containers (read-only)
            | "docx" | "hwpx" | "pdf"
        )
    )
}

/// Read any supported document as displayable text. Plain-text formats are
/// read directly; container formats are converted to Markdown.
pub fn read_document(path: &Path) -> io::Result<String> {
    let text = match ext_lowercase(path).as_deref() {
        Some("docx") => docx_to_markdown(path)?,
        Some("hwpx") => hwpx_to_markdown(path)?,
        // Real page rendering via CoreGraphics; text extraction is the
        // fallback for PDFs CoreGraphics can't open.
        Some("pdf") => crate::pdf::pdf_to_markdown_pages(path).or_else(|_| pdf_to_text(path))?,
        _ => return std::fs::read_to_string(path),
    };
    Ok(truncate_converted(text))
}

fn ext_lowercase(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

fn invalid(msg: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn truncate_converted(text: String) -> String {
    if text.len() <= IMPORT_MAX_BYTES {
        return text;
    }
    let mut cut = IMPORT_MAX_BYTES;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n\n> ⚠️ *Document truncated for display.*", &text[..cut])
}

fn read_zip_entry(path: &Path, name: &str) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(invalid)?;
    let mut xml = String::new();
    archive.by_name(name).map_err(invalid)?.read_to_string(&mut xml)?;
    Ok(xml)
}

// ---------------------------------------------------------------- docx

fn docx_to_markdown(path: &Path) -> io::Result<String> {
    let xml = read_zip_entry(path, "word/document.xml")?;
    Ok(docx_xml_to_markdown(&xml))
}

/// Single-pass OOXML → Markdown: headings, bold/italic runs, list items,
/// and tables. Anything fancier degrades to plain paragraphs.
fn docx_xml_to_markdown(xml: &str) -> String {
    let mut reader = Reader::from_str(xml);

    let mut out = String::new();
    let mut para = String::new();
    let mut heading = 0usize;
    let mut is_list = false;
    let mut prev_was_list = false;

    let mut run = String::new();
    let mut bold = false;
    let mut italic = false;
    let mut in_run_props = false;
    let mut in_para_props = false;
    let mut in_text = false;

    // Rows of cells for the outermost table; deeper tables linearize.
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut table_depth = 0usize;

    loop {
        match reader.read_event() {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"w:p" => {
                    para.clear();
                    heading = 0;
                    is_list = false;
                }
                b"w:pPr" => in_para_props = true,
                b"w:rPr" => in_run_props = true,
                b"w:r" => {
                    run.clear();
                    bold = false;
                    italic = false;
                }
                b"w:t" => in_text = true,
                b"w:numPr" => is_list = true,
                b"w:tbl" => {
                    table_depth += 1;
                    if table_depth == 1 {
                        table = Some(Vec::new());
                    }
                }
                b"w:tr" if table_depth == 1 => {
                    if let Some(rows) = table.as_mut() {
                        rows.push(Vec::new());
                    }
                }
                b"w:tc" if table_depth == 1 => {
                    if let Some(row) = table.as_mut().and_then(|r| r.last_mut()) {
                        row.push(String::new());
                    }
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.name().as_ref() {
                b"w:pStyle" => {
                    if let Ok(Some(attr)) = e.try_get_attribute("w:val") {
                        let style = String::from_utf8_lossy(&attr.value).to_string();
                        heading = heading_level(&style);
                    }
                }
                b"w:numPr" => is_list = true,
                b"w:b" if in_run_props && !in_para_props => bold = true,
                b"w:i" if in_run_props && !in_para_props => italic = true,
                b"w:br" => run.push('\n'),
                b"w:tab" => run.push('\t'),
                _ => {}
            },
            Ok(Event::Text(t))
                if in_text => {
                    run.push_str(&t.xml_content().unwrap_or_default());
                }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"w:t" => in_text = false,
                b"w:pPr" => in_para_props = false,
                b"w:rPr" => in_run_props = false,
                b"w:r"
                    if !run.is_empty() => {
                        para.push_str(&styled_run(&run, bold, italic));
                    }
                b"w:p" => {
                    let line = para.trim().to_string();
                    if let Some(cell) =
                        table.as_mut().and_then(|r| r.last_mut()).and_then(|r| r.last_mut())
                    {
                        if !line.is_empty() {
                            if !cell.is_empty() {
                                cell.push(' ');
                            }
                            cell.push_str(&line);
                        }
                    } else if !line.is_empty() {
                        if is_list {
                            out.push_str("- ");
                            out.push_str(&line);
                            out.push('\n');
                        } else {
                            if prev_was_list {
                                out.push('\n');
                            }
                            for _ in 0..heading.min(6) {
                                out.push('#');
                            }
                            if heading > 0 {
                                out.push(' ');
                            }
                            out.push_str(&line);
                            out.push_str("\n\n");
                        }
                        prev_was_list = is_list;
                    }
                }
                b"w:tbl" => {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0
                        && let Some(rows) = table.take() {
                            push_markdown_table(&mut out, &rows);
                        }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn heading_level(style: &str) -> usize {
    if style.eq_ignore_ascii_case("Title") {
        return 1;
    }
    style
        .strip_prefix("Heading")
        .or_else(|| style.strip_prefix("heading"))
        .and_then(|n| n.trim().parse::<usize>().ok())
        .filter(|n| (1..=6).contains(n))
        .unwrap_or(0)
}

fn styled_run(text: &str, bold: bool, italic: bool) -> String {
    // Only wrap trimmed content; markdown emphasis breaks across spaces.
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return text.to_string();
    }
    match (bold, italic) {
        (true, true) => format!("***{trimmed}*** "),
        (true, false) => format!("**{trimmed}** "),
        (false, true) => format!("*{trimmed}* "),
        (false, false) => text.to_string(),
    }
}

fn push_markdown_table(out: &mut String, rows: &[Vec<String>]) {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols == 0 {
        return;
    }
    for (i, row) in rows.iter().enumerate() {
        out.push('|');
        for c in 0..cols {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            out.push(' ');
            out.push_str(&cell.replace('|', "\\|"));
            out.push_str(" |");
        }
        out.push('\n');
        if i == 0 {
            out.push('|');
            for _ in 0..cols {
                out.push_str(" --- |");
            }
            out.push('\n');
        }
    }
    out.push('\n');
}

// ---------------------------------------------------------------- hwpx

fn hwpx_to_markdown(path: &Path) -> io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(invalid)?;

    let mut sections: Vec<String> = archive
        .file_names()
        .filter(|n| n.starts_with("Contents/section") && n.ends_with(".xml"))
        .map(str::to_string)
        .collect();
    sections.sort();
    if sections.is_empty() {
        return Err(invalid("no Contents/section*.xml in HWPX archive"));
    }

    let mut out = String::new();
    for name in sections {
        let mut xml = String::new();
        archive.by_name(&name).map_err(invalid)?.read_to_string(&mut xml)?;
        hwpx_section_to_text(&xml, &mut out);
    }
    Ok(out)
}

/// Extract paragraph text (`<hp:t>` inside `<hp:p>`); tables linearize.
fn hwpx_section_to_text(xml: &str, out: &mut String) {
    let mut reader = Reader::from_str(xml);
    let mut in_text = false;
    let mut para = String::new();
    loop {
        match reader.read_event() {
            Err(_) | Ok(Event::Eof) => break,
            Ok(Event::Start(e))
                if e.name().as_ref() == b"hp:t" => {
                    in_text = true;
                }
            Ok(Event::Text(t))
                if in_text => {
                    para.push_str(&t.xml_content().unwrap_or_default());
                }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"hp:t" => in_text = false,
                b"hp:p" => {
                    let line = para.trim();
                    if !line.is_empty() {
                        out.push_str(line);
                        out.push_str("\n\n");
                    }
                    para.clear();
                }
                _ => {}
            },
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- pdf

fn pdf_to_text(path: &Path) -> io::Result<String> {
    let owned = path.to_path_buf();
    // pdf-extract can panic on malformed files; contain it.
    match std::panic::catch_unwind(move || pdf_extract::extract_text(&owned)) {
        Ok(Ok(text)) => Ok(collapse_blank_lines(&text)),
        Ok(Err(err)) => Err(invalid(err)),
        Err(_) => Err(invalid("PDF text extraction failed")),
    }
}

/// Squash runs of 3+ newlines down to a paragraph break.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(ch);
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{
        IMPORT_MAX_BYTES, collapse_blank_lines, ext_lowercase, is_imported_doc, is_supported_doc,
        push_markdown_table, styled_run, truncate_converted,
    };
    use std::path::Path;

    #[test]
    fn ext_lowercase_normalizes() {
        assert_eq!(ext_lowercase(Path::new("A.PDF")).as_deref(), Some("pdf"));
        assert_eq!(ext_lowercase(Path::new("a.Md")).as_deref(), Some("md"));
        assert_eq!(ext_lowercase(Path::new("noext")), None);
    }

    #[test]
    fn truncate_converted_passes_short_text() {
        let s = "small document".to_string();
        assert_eq!(truncate_converted(s.clone()), s);
    }

    #[test]
    fn truncate_converted_caps_and_notes_long_text() {
        let big = "a".repeat(IMPORT_MAX_BYTES + 5_000);
        let out = truncate_converted(big);
        assert!(out.len() < IMPORT_MAX_BYTES + 200);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn truncate_converted_respects_char_boundaries() {
        // multi-byte chars right at the cap must not panic or split mid-codepoint.
        let big = "가".repeat(IMPORT_MAX_BYTES); // 3 bytes each
        let out = truncate_converted(big);
        assert!(out.contains("truncated"));
    }

    #[test]
    fn styled_run_wraps_emphasis() {
        assert_eq!(styled_run("hi", true, false), "**hi** ");
        assert_eq!(styled_run("hi", false, true), "*hi* ");
        assert_eq!(styled_run("hi", true, true), "***hi*** ");
        assert_eq!(styled_run("hi", false, false), "hi");
        assert_eq!(styled_run("   ", true, true), "   "); // blank passes through
    }

    #[test]
    fn markdown_table_has_header_separator_and_escapes_pipes() {
        let mut out = String::new();
        push_markdown_table(&mut out, &[vec!["a".into(), "b|c".into()], vec!["1".into()]]);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "| a | b\\|c |"); // pipe escaped, padded to 2 cols
        assert_eq!(lines[1], "| --- | --- |"); // separator after header row
        assert_eq!(lines[2], "| 1 |  |"); // missing cell rendered empty
    }

    #[test]
    fn supported_extensions_accepted() {
        for p in [
            "a.md", "a.markdown", "a.txt", "a.json", "a.rs", "a.tsx", "a.yaml", "a.toml",
            "a.docx", "a.hwpx", "a.pdf",
        ] {
            assert!(is_supported_doc(Path::new(p)), "{p} should be supported");
        }
    }

    #[test]
    fn supported_is_case_insensitive() {
        assert!(is_supported_doc(Path::new("README.MD")));
        assert!(is_supported_doc(Path::new("deck.PDF")));
    }

    #[test]
    fn unsupported_extensions_rejected() {
        for p in ["a.exe", "a.png", "a.zip", "noext", "a.docxx"] {
            assert!(!is_supported_doc(Path::new(p)), "{p} should be unsupported");
        }
    }

    #[test]
    fn imported_are_converter_formats_only() {
        for p in ["a.docx", "a.hwpx", "a.pdf", "DECK.PDF"] {
            assert!(is_imported_doc(Path::new(p)), "{p} converts");
        }
        for p in ["a.md", "a.json", "a.txt"] {
            assert!(!is_imported_doc(Path::new(p)), "{p} is read directly");
        }
    }

    #[test]
    fn collapse_blank_lines_caps_at_one_blank() {
        assert_eq!(collapse_blank_lines("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(collapse_blank_lines("a\nb"), "a\nb");
        assert_eq!(collapse_blank_lines("a\n\nb"), "a\n\nb");
    }
}
