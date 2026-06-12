//! PDF → page PNGs via CoreGraphics (native macOS rendering, full fidelity).
//!
//! Pages are rasterized into a cache directory under the system temp dir and
//! the document becomes Markdown image links the preview renders like any
//! other document. Safe to run on the background executor (everything is
//! created and dropped inside one call).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use objc2_core_foundation::{CFRetained, CFString, CFURL, CFURLPathStyle, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGPDFBox, CGPDFDocument,
    CGPDFPage,
};

/// Rendering scale relative to PDF points (2x ≈ retina-crisp).
const SCALE: f64 = 2.0;
/// Don't rasterize monster documents past this many pages.
const MAX_PAGES: usize = 80;

fn invalid(msg: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

/// Render `path` to page images and return Markdown that displays them.
pub fn pdf_to_markdown_pages(path: &Path) -> io::Result<String> {
    let doc = open_document(path)?;
    let page_count = CGPDFDocument::number_of_pages(Some(&doc));
    if page_count == 0 {
        return Err(invalid("PDF has no pages"));
    }

    let dir = cache_dir_for(path)?;
    let shown = page_count.min(MAX_PAGES);

    let mut md = String::new();
    for n in 1..=shown {
        let png = dir.join(format!("page-{n:03}.png"));
        if !png.exists() {
            render_page(&doc, n, &png)?;
        }
        md.push_str(&format!("![Page {n}]({})\n\n", png.display()));
    }
    if shown < page_count {
        md.push_str(&format!(
            "> ⚠️ *Showing the first {shown} of {page_count} pages.*\n"
        ));
    }
    Ok(md)
}

fn open_document(path: &Path) -> io::Result<CFRetained<CGPDFDocument>> {
    let cf_path = CFString::from_str(&path.to_string_lossy());
    let url = CFURL::with_file_system_path(
        None,
        Some(&cf_path),
        CFURLPathStyle::CFURLPOSIXPathStyle,
        false,
    )
    .ok_or_else(|| invalid("invalid path"))?;
    let doc =
        CGPDFDocument::with_url(Some(&url)).ok_or_else(|| invalid("not a readable PDF"))?;
    if CGPDFDocument::is_encrypted(Some(&doc)) && !CGPDFDocument::is_unlocked(Some(&doc)) {
        return Err(invalid("PDF is password-protected"));
    }
    Ok(doc)
}

/// Cache directory keyed by path + size + mtime, so edits invalidate it.
fn cache_dir_for(path: &Path) -> io::Result<PathBuf> {
    let meta = std::fs::metadata(path)?;
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    meta.len().hash(&mut hasher);
    if let Ok(modified) = meta.modified() {
        modified.hash(&mut hasher);
    }
    let dir = std::env::temp_dir()
        .join("markforge-pdf")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn render_page(doc: &CGPDFDocument, number: usize, out: &Path) -> io::Result<()> {
    let page = CGPDFDocument::page(Some(doc), number)
        .ok_or_else(|| invalid(format!("missing page {number}")))?;
    let crop = CGPDFPage::box_rect(Some(&page), CGPDFBox::CropBox);
    let rotated = CGPDFPage::rotation_angle(Some(&page)).rem_euclid(180) != 0;
    let (pt_w, pt_h) = if rotated {
        (crop.size.height, crop.size.width)
    } else {
        (crop.size.width, crop.size.height)
    };
    let width = (pt_w * SCALE).round().max(1.0) as usize;
    let height = (pt_h * SCALE).round().max(1.0) as usize;

    let mut buf = vec![0u8; width * height * 4];
    let space = CGColorSpace::new_device_rgb().ok_or_else(|| invalid("no RGB color space"))?;
    let ctx = unsafe {
        CGBitmapContextCreate(
            buf.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(&space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }
    .ok_or_else(|| invalid("couldn't create bitmap context"))?;

    // White page background, then let CoreGraphics fit/rotate the page.
    CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);
    let target = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: width as f64,
            height: height as f64,
        },
    };
    CGContext::fill_rect(Some(&ctx), target);
    let transform = CGPDFPage::drawing_transform(Some(&page), CGPDFBox::CropBox, target, 0, true);
    CGContext::concat_ctm(Some(&ctx), transform);
    CGContext::draw_pdf_page(Some(&ctx), Some(&page));
    drop(ctx); // flush writes into `buf` before reading it

    write_png(out, width, height, &buf)
}

fn write_png(out: &Path, width: usize, height: usize, rgba: &[u8]) -> io::Result<()> {
    let file = std::fs::File::create(out)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(invalid)?;
    writer.write_image_data(rgba).map_err(invalid)?;
    Ok(())
}
