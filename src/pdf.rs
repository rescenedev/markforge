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

/// Target rasterized page width (px). Wide enough to fill the preview pane on a
/// typical retina window so pages are never shown narrower than their column.
/// We scale every page to this width regardless of its point size — some decks
/// author pages in tiny units (tens of points), and a fixed scale cap would
/// leave them a few hundred pixels wide, floating in blank space.
const TARGET_WIDTH: f64 = 2400.0;
/// A trimmed crop at least this fraction of `TARGET_WIDTH` already fills the
/// preview; only narrower crops get upscaled (which costs a full resample).
const FILL_MIN: f64 = 0.9;
/// Hard ceiling on either rasterized dimension, so a very tall/narrow page
/// (long receipts, banners) can't balloon the bitmap. Bounds memory at
/// `TARGET_WIDTH × MAX_DIM × 4` bytes.
const MAX_DIM: f64 = 6000.0;
/// Don't rasterize monster documents past this many pages.
const MAX_PAGES: usize = 80;

/// A page channel ≥ this (out of 255) counts as blank background when trimming.
const BG_LEVEL: u8 = 248;
/// Auto-trim a page when its content covers less than this fraction along
/// *either* axis — protects normal documents whose white margins are
/// intentional (a Letter page with 1″ margins fills ~0.76 each way), while
/// rescuing slides that float content in a sea of white (cards near 0.4, or a
/// title block above a half-empty slide).
const TRIM_BELOW: f64 = 0.7;
/// Breathing room kept around trimmed content, as a fraction of its size.
const TRIM_PAD: f64 = 0.015;
/// When locating the content block, bridge gaps up to this fraction of the axis
/// (line spacing, the gap between a title and its body). Larger gaps split the
/// page into separate blocks.
const CONTENT_GAP: f64 = 0.06;
/// A content block this far below the densest block (by ink) is treated as a
/// stray (page number, footer rule, edge decoration) and excluded from bounds.
const STRAY_INK_PCT: u64 = 15;

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
    // Bump the version segment when the render parameters change so stale
    // (differently-scaled) page images are regenerated.
    let dir = std::env::temp_dir()
        .join("markforge-pdf")
        .join("v3")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Content bounds within a page, as fractions of width/height measured from the
/// top-left of the rendered image. `(0,0,1,1)` means the content fills the page.
struct Bounds {
    fx0: f64,
    fy0: f64,
    fx1: f64,
    fy1: f64,
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
    let pt_w = pt_w.max(1.0);
    let pt_h = pt_h.max(1.0);

    // Render the full page at target scale, then look at the actual pixels to
    // find the ink. Slide decks often float a small card in a huge white page;
    // trimming that whitespace lets the real content fill the preview instead
    // of hiding in the margins. Detecting on the same pixels we'd otherwise
    // ship keeps the decision honest (a cheap low-res probe disagreed with the
    // final render and mis-trimmed).
    // Scale to TARGET_WIDTH, but never let either side exceed MAX_DIM.
    let scale = (TARGET_WIDTH / pt_w).min(MAX_DIM / pt_w).min(MAX_DIM / pt_h);
    let width = (pt_w * scale).round().max(1.0) as usize;
    let height = (pt_h * scale).round().max(1.0) as usize;
    let full_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: width as f64,
            height: height as f64,
        },
    };
    let full = rasterize(&page, width, height, full_rect)?;

    // Keep normal documents (whose margins are intentional) untouched; trim a
    // page when its content leaves most of either axis empty.
    let trim = content_bounds(&full, width, height)
        .filter(|b| (b.fx1 - b.fx0) < TRIM_BELOW || (b.fy1 - b.fy0) < TRIM_BELOW);
    let Some(b) = trim else {
        return write_png(out, width, height, &full);
    };

    // Pad the content box slightly so the trim keeps a little breathing room.
    let pad_x = (b.fx1 - b.fx0) * TRIM_PAD;
    let pad_y = (b.fy1 - b.fy0) * TRIM_PAD;
    let fx0 = (b.fx0 - pad_x).max(0.0);
    let fy0 = (b.fy0 - pad_y).max(0.0);
    let fx1 = (b.fx1 + pad_x).min(1.0);
    let fy1 = (b.fy1 + pad_y).min(1.0);

    // Slice the content region straight out of the page we already rendered.
    // (Re-rendering a zoomed sub-region via the CoreGraphics transform proved
    // unreliable — `drawing_transform` doesn't fill an enlarged bitmap the way
    // a plain scale would — so we crop pixels, which is exact.) The full render
    // is wide (TARGET_WIDTH) so even a trimmed slice stays reasonably crisp.
    let x0 = ((fx0 * width as f64).floor() as usize).min(width - 1);
    let y0 = ((fy0 * height as f64).floor() as usize).min(height - 1);
    let x1 = ((fx1 * width as f64).ceil() as usize).clamp(x0 + 1, width);
    let y1 = ((fy1 * height as f64).ceil() as usize).clamp(y0 + 1, height);
    let cw = x1 - x0;
    let ch = y1 - y0;
    let mut cropped = vec![0u8; cw * ch * 4];
    for row in 0..ch {
        let src = ((y0 + row) * width + x0) * 4;
        let dst = row * cw * 4;
        cropped[dst..dst + cw * 4].copy_from_slice(&full[src..src + cw * 4]);
    }

    // Widen narrow content (a lone title, a divider) so it fills the preview
    // instead of sitting in a column with blank space to its right. Pages that
    // already span most of the target width are left alone — upscaling them
    // would cost a full-frame resample for no visible gain. Upscaling a tight
    // crop softens it a touch, but a filled column reads far better than a gap.
    if (cw as f64) < TARGET_WIDTH * FILL_MIN {
        let dw = TARGET_WIDTH as usize;
        let dh = (ch * dw / cw).max(1);
        let scaled = upscale_rgba(&cropped, cw, ch, dw, dh);
        return write_png(out, dw, dh, &scaled);
    }
    write_png(out, cw, ch, &cropped)
}

/// Scan an RGBA buffer (top-left origin) for the content block, by row/column
/// ink density. Returns `None` for a blank page.
///
/// Two refinements keep this honest on real slides:
/// 1. A row/column only counts as inked once ≥0.5% of its pixels are non-white,
///    so a sparse speckle (faint full-page backgrounds, scan noise) is ignored.
/// 2. Bounds track the *main* content block, not the raw extremes. Inked
///    rows/cols are grouped into blocks (bridging small gaps); blocks far less
///    inked than the densest one — page numbers, footer rules, a lone band at a
///    slide's edge — are dropped. This stops a stray footer from pinning the
///    bounds to the full page and defeating the trim.
fn content_bounds(buf: &[u8], w: usize, h: usize) -> Option<Bounds> {
    let is_ink = |i: usize| {
        let (r, g, b, a) = (buf[i], buf[i + 1], buf[i + 2], buf[i + 3]);
        a >= 8 && (r < BG_LEVEL || g < BG_LEVEL || b < BG_LEVEL)
    };

    let mut col_ink = vec![0u32; w];
    let mut row_ink = vec![0u32; h];
    for (y, row_count) in row_ink.iter_mut().enumerate() {
        let row = y * w * 4;
        for (x, col_count) in col_ink.iter_mut().enumerate() {
            if is_ink(row + x * 4) {
                *col_count += 1;
                *row_count += 1;
            }
        }
    }

    let col_thresh = ((h as f64 * 0.005) as u32).max(3);
    let row_thresh = ((w as f64 * 0.005) as u32).max(3);
    let (min_x, max_x) = content_span(&col_ink, col_thresh)?;
    let (min_y, max_y) = content_span(&row_ink, row_thresh)?;

    Some(Bounds {
        fx0: min_x as f64 / w as f64,
        fy0: min_y as f64 / h as f64,
        fx1: max_x as f64 / w as f64,
        fy1: max_y as f64 / h as f64,
    })
}

/// Given per-line ink counts along one axis, return the `[start, end)` span of
/// the main content. Lines ≥ `thresh` are grouped into blocks, bridging gaps up
/// to `CONTENT_GAP` of the axis; blocks with under `STRAY_INK_PCT`% of the
/// densest block's ink are dropped as strays. The span covers the kept blocks.
fn content_span(ink: &[u32], thresh: u32) -> Option<(usize, usize)> {
    let n = ink.len();
    let max_gap = (n as f64 * CONTENT_GAP) as usize;

    // Collect blocks of inked lines, merging runs separated by a small gap.
    let mut blocks: Vec<(usize, usize, u64)> = Vec::new(); // start, end_excl, ink
    let mut i = 0;
    while i < n {
        if ink[i] < thresh {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i + 1;
        let mut gap = 0;
        let mut j = i + 1;
        while j < n {
            if ink[j] >= thresh {
                end = j + 1;
                gap = 0;
            } else {
                gap += 1;
                if gap > max_gap {
                    break;
                }
            }
            j += 1;
        }
        let sum: u64 = ink[start..end].iter().map(|&v| v as u64).sum();
        blocks.push((start, end, sum));
        i = end;
    }

    let densest = blocks.iter().map(|b| b.2).max()?;
    let kept = blocks
        .iter()
        .filter(|b| b.2 * 100 >= densest * STRAY_INK_PCT);
    let start = kept.clone().map(|b| b.0).min()?;
    let end = kept.map(|b| b.1).max()?;
    Some((start, end))
}

/// Rasterize `page` into a `width`×`height` RGBA buffer, fitting the crop box
/// into `fit` (normally the full bitmap). Buffer is top-left origin, RGBA8.
fn rasterize(page: &CGPDFPage, width: usize, height: usize, fit: CGRect) -> io::Result<Vec<u8>> {
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
    CGContext::fill_rect(
        Some(&ctx),
        CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: width as f64,
                height: height as f64,
            },
        },
    );
    let transform = CGPDFPage::drawing_transform(Some(page), CGPDFBox::CropBox, fit, 0, true);
    CGContext::concat_ctm(Some(&ctx), transform);
    CGContext::draw_pdf_page(Some(&ctx), Some(page));
    drop(ctx); // flush writes into `buf` before reading it
    Ok(buf)
}

/// Bilinear upscale of a top-left-origin RGBA buffer to `dw`×`dh`.
fn upscale_rgba(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dw * dh * 4];
    let fx = sw as f64 / dw as f64;
    let fy = sh as f64 / dh as f64;
    for dy in 0..dh {
        let sy = ((dy as f64 + 0.5) * fy - 0.5).max(0.0);
        let sy0 = (sy.floor() as usize).min(sh - 1);
        let sy1 = (sy0 + 1).min(sh - 1);
        let wy = sy - sy0 as f64;
        for dx in 0..dw {
            let sx = ((dx as f64 + 0.5) * fx - 0.5).max(0.0);
            let sx0 = (sx.floor() as usize).min(sw - 1);
            let sx1 = (sx0 + 1).min(sw - 1);
            let wx = sx - sx0 as f64;
            let d = (dy * dw + dx) * 4;
            let (a, b, c, e) = (
                (sy0 * sw + sx0) * 4,
                (sy0 * sw + sx1) * 4,
                (sy1 * sw + sx0) * 4,
                (sy1 * sw + sx1) * 4,
            );
            for k in 0..4 {
                let top = src[a + k] as f64 * (1.0 - wx) + src[b + k] as f64 * wx;
                let bot = src[c + k] as f64 * (1.0 - wx) + src[e + k] as f64 * wx;
                dst[d + k] = (top * (1.0 - wy) + bot * wy).round() as u8;
            }
        }
    }
    dst
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
