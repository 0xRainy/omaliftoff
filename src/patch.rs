//! Mission patch → half-block ("▀") art. Works in every truecolor terminal
//! (no kitty/sixel needed); transparent pixels stay transparent.
use crate::config::paths;
use anyhow::{anyhow, Result};
use image::{imageops::FilterType, RgbaImage};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use std::io::Read;
use std::path::PathBuf;

fn file_for(url: &str) -> PathBuf {
    let h = url.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    paths::cache_dir().join("patches").join(format!("{h:016x}"))
}

/// Download (once, cached on disk; max 4 MB) and decode.
pub fn load(url: &str) -> Result<RgbaImage> {
    let p = file_for(url);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(_) => {
            let r = ureq::get(url).timeout(std::time::Duration::from_secs(15)).call().map_err(|e| anyhow!("{e}"))?;
            let mut b = vec![];
            r.into_reader().take(4 << 20).read_to_end(&mut b)?;
            crate::store::write_atomic(&p, &b)?;
            b
        }
    };
    Ok(image::load_from_memory(&bytes)?.to_rgba8())
}

/// Fit into `cols` x `rows` cells (each cell = 1x2 pixels).
pub fn render(img: &RgbaImage, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let (w, h) = (img.width().max(1) as f64, img.height().max(1) as f64);
    let scale = (cols as f64 / w).min((rows as f64 * 2.0) / h);
    let (tw, th) = (((w * scale).round() as u32).max(1), ((h * scale).round() as u32).max(2));
    let small = image::imageops::resize(img, tw, th, FilterType::Triangle);
    let px = |x: u32, y: u32| -> Option<Color> {
        if y >= th {
            return None;
        }
        let p = small.get_pixel(x, y).0;
        (p[3] >= 96).then(|| Color::Rgb(p[0], p[1], p[2]))
    };
    (0..th.div_ceil(2))
        .map(|r| {
            Line::from(
                (0..tw)
                    .map(|x| {
                        let (top, bot) = (px(x, r * 2), px(x, r * 2 + 1));
                        match (top, bot) {
                            (None, None) => Span::raw(" "),
                            (Some(t), None) => Span::styled("▀", Style::default().fg(t)),
                            (None, Some(b)) => Span::styled("▄", Style::default().fg(b)),
                            (Some(t), Some(b)) => Span::styled("▀", Style::default().fg(t).bg(b)),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_within_bounds_and_keeps_transparency() {
        let mut img = RgbaImage::new(40, 40);
        for (x, y, p) in img.enumerate_pixels_mut() {
            *p = if x < 20 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 0, 0]) };
            let _ = y;
        }
        let lines = render(&img, 10, 5);
        assert!(lines.len() <= 5);
        assert!(lines.iter().all(|l| l.width() <= 10));
        let last = &lines[0].spans.last().unwrap();
        assert_eq!(last.content, " ");
    }
}
