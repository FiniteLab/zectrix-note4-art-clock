//! 400×300 grayscale canvas with outline-font text rendering.
//!
//! The SDK dithers whatever it receives, so text must be drawn in pure black on
//! a flat background to stay sharp: grey ink turns into dither noise on the panel.
use ab_glyph::{point, Font, FontVec, GlyphId, PxScale, ScaleFont};
use image::{codecs::png::PngEncoder, imageops, ExtendedColorType, GrayImage, ImageEncoder, Luma};
use std::path::Path;

pub const W: u32 = 400;
pub const H: u32 = 300;

pub struct Face(FontVec);

impl Face {
    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        FontVec::try_from_vec(bytes).map(Face).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{}: {e}", path.display()),
            )
        })
    }

    /// ab_glyph scales by ascent minus descent; convert so `px` is the em size,
    /// which is what the 12 px pixel font needs to land exactly on the pixel grid.
    fn scale(&self, px: f32) -> PxScale {
        let upem = self.0.units_per_em().unwrap_or(1000.0);
        PxScale::from(px * self.0.height_unscaled() / upem)
    }

    pub fn width(&self, px: f32, text: &str) -> f32 {
        let sf = self.0.as_scaled(self.scale(px));
        let mut w = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in text.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                w += sf.kern(p, id);
            }
            w += sf.h_advance(id);
            prev = Some(id);
        }
        w
    }
}

/// A font at a size, drawn either antialiased or snapped to whole pixels.
#[derive(Clone, Copy)]
pub struct Text<'a> {
    pub face: &'a Face,
    pub px: f32,
    pub antialias: bool,
}

impl<'a> Text<'a> {
    pub fn smooth(face: &'a Face, px: f32) -> Self {
        Self { face, px, antialias: true }
    }

    /// For bitmap-style pixel fonts used at their native size.
    pub fn crisp(face: &'a Face, px: f32) -> Self {
        Self { face, px, antialias: false }
    }

    pub fn width(&self, s: &str) -> f32 {
        self.face.width(self.px, s)
    }

    /// Shortens `s` with a trailing ellipsis until it fits in `max_w`.
    pub fn fit(&self, s: &str, max_w: f32) -> String {
        if self.width(s) <= max_w {
            return s.to_string();
        }
        let mut chars: Vec<char> = s.chars().collect();
        while chars.pop().is_some() {
            let candidate: String = chars.iter().collect::<String>() + "…";
            if self.width(&candidate) <= max_w {
                return candidate;
            }
        }
        String::new()
    }

    /// Breaks `s` into lines no wider than `max_w`. Chinese breaks between any two
    /// characters, Latin text between words; closing punctuation never starts a line
    /// and opening punctuation never ends one. `\n` forces a break.
    pub fn wrap(&self, s: &str, max_w: f32) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut line = String::new();
            for (space, atom) in atoms(para) {
                let joined = match (line.is_empty(), space) {
                    (true, _) => atom.clone(),
                    (false, true) => format!("{line} {atom}"),
                    (false, false) => format!("{line}{atom}"),
                };
                if self.width(&joined) <= max_w {
                    line = joined;
                    continue;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                if self.width(&atom) <= max_w {
                    line = atom;
                    continue;
                }
                // A single word wider than the line: break it anywhere except
                // before closing punctuation, which may overhang instead.
                for c in atom.chars() {
                    let mut next = line.clone();
                    next.push(c);
                    if !line.is_empty() && self.width(&next) > max_w && !CLOSING.contains(c) {
                        lines.push(std::mem::replace(&mut line, c.to_string()));
                    } else {
                        line = next;
                    }
                }
            }
            lines.push(line);
        }
        lines
    }
}

/// Never at the start of a line.
const CLOSING: &str = "，。、；：？！）」』》〉】〕”’…—～·％,.;:!?)]}%";
/// Never at the end of a line.
const OPENING: &str = "（「『《〈【〔“‘([{";

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x9FFF | 0xF900..=0xFAFF | 0xFE30..=0xFE4F | 0xFF00..=0xFFEF | 0x20000..=0x3FFFF)
}

/// Splits a paragraph into pieces that must stay on one line, each flagged with
/// whether whitespace came before it.
fn atoms(s: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    let mut space = false;
    // The previous piece is a Latin word that the next Latin character continues.
    let mut word = false;
    // The previous piece ends with an opening bracket.
    let mut open = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
            word = false;
            continue;
        }
        let latin = !is_cjk(c) && !CLOSING.contains(c) && !OPENING.contains(c);
        let attach = match out.last() {
            None => false,
            Some(_) => CLOSING.contains(c) || (!space && (open || (latin && word))),
        };
        if attach {
            let last = out.last_mut().expect("attach implies a previous piece");
            if space {
                last.1.push(' ');
            }
            last.1.push(c);
        } else {
            out.push((space, c.to_string()));
        }
        space = false;
        word = !is_cjk(c);
        open = OPENING.contains(c);
    }
    out
}

pub struct Canvas(pub GrayImage);

impl Canvas {
    pub fn new(background: u8) -> Self {
        Canvas(GrayImage::from_pixel(W, H, Luma([background])))
    }

    pub fn paste(&mut self, img: &GrayImage, x: i64, y: i64) {
        imageops::replace(&mut self.0, img, x, y);
    }

    pub fn hline(&mut self, x0: u32, x1: u32, y: u32, value: u8) {
        if y >= H {
            return;
        }
        for x in x0..=x1.min(W - 1) {
            self.0.put_pixel(x, y, Luma([value]));
        }
    }

    pub fn vline(&mut self, x: u32, y0: u32, y1: u32, value: u8) {
        if x >= W {
            return;
        }
        for y in y0..=y1.min(H - 1) {
            self.0.put_pixel(x, y, Luma([value]));
        }
    }

    /// Draws `s` with its baseline at `baseline`, starting at `x`. Returns the end x.
    pub fn text(&mut self, style: Text, x: f32, baseline: f32, s: &str, ink: u8) -> f32 {
        let scale = style.face.scale(style.px);
        let font = &style.face.0;
        let sf = font.as_scaled(scale);
        let img = &mut self.0;
        let mut caret = x;
        let mut prev: Option<GlyphId> = None;
        for ch in s.chars() {
            let id = sf.glyph_id(ch);
            if let Some(p) = prev {
                caret += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(scale, point(caret, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let bounds = outline.px_bounds();
                outline.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if px < 0 || py < 0 || px >= W as i32 || py >= H as i32 {
                        return;
                    }
                    let a = if style.antialias {
                        coverage.clamp(0.0, 1.0)
                    } else if coverage >= 0.5 {
                        1.0
                    } else {
                        0.0
                    };
                    if a <= 0.0 {
                        return;
                    }
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    p.0[0] = (p.0[0] as f32 * (1.0 - a) + ink as f32 * a).round() as u8;
                });
            }
            caret += sf.h_advance(id);
            prev = Some(id);
        }
        caret
    }

    pub fn text_fit(&mut self, style: Text, x: f32, baseline: f32, s: &str, max_w: f32, ink: u8) {
        let fitted = style.fit(s, max_w);
        self.text(style, x, baseline, &fitted, ink);
    }

    /// Rows that contain anything other than `background`.
    pub fn ink_rows(&self, background: u8) -> Vec<u32> {
        (0..H)
            .filter(|&y| (0..W).any(|x| self.0.get_pixel(x, y).0[0] != background))
            .collect()
    }

    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(self.0.as_raw(), W, H, ExtendedColorType::L8)
            .expect("encoding an in-memory grayscale PNG cannot fail");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sans() -> Face {
        Face::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/SourceHanSansCN-Regular.otf"))
            .unwrap()
    }

    #[test]
    fn wrapped_lines_fit_and_keep_every_character() {
        let face = sans();
        let style = Text::smooth(&face, 16.0);
        let text = "这幅木版画出自葛饰北斋的组画《富岳三十六景》，作于他七十岁上下。Mount Fuji is the protagonist in each scene, viewed from afar or up close.";
        let lines = style.wrap(text, 200.0);
        assert!(lines.len() > 3);
        for line in &lines {
            assert!(style.width(line) <= 200.0, "{line}");
        }
        let squash = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        assert_eq!(squash(&lines.concat()), squash(text));
    }

    #[test]
    fn punctuation_and_words_stay_together() {
        let face = sans();
        let style = Text::smooth(&face, 16.0);
        // From the width of the longest word up; narrower lines must split words.
        for w in (100..=240).step_by(7) {
            for line in style.wrap("画面中有夏季、柏木属、风景画。“引号”（括号）Sharkey’s boxing ring, 1909.", w as f32) {
                let first = line.chars().next().unwrap_or(' ');
                let last = line.chars().last().unwrap_or(' ');
                assert!(!CLOSING.contains(first), "line starts with {first}: {line}");
                assert!(!OPENING.contains(last), "line ends with {last}: {line}");
                assert!(!line.starts_with("s ") && !line.starts_with("oxing"), "word split: {line}");
            }
        }
    }

    #[test]
    fn explicit_newlines_break() {
        let face = sans();
        let style = Text::smooth(&face, 16.0);
        assert_eq!(style.wrap("第一段\n第二段", 300.0), vec!["第一段", "第二段"]);
    }
}
