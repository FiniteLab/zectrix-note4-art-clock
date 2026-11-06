//! Artwork images and metadata, and the processing that fits them to the e-ink panel.
use crate::render::{H, W};
use image::{
    imageops::{self, FilterType},
    GrayImage, Luma,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Pictures narrower than the art band are cropped to fill it, and those at least
/// this wide keep enough of themselves whatever they show.
pub const MIN_WIDE_ASPECT: f32 = 1.2;
/// Upright pictures down to this aspect keep well under half their height, which
/// scenery survives but a figure does not.
pub const MIN_UPRIGHT_ASPECT: f32 = 0.75;
/// Pictures wider than the band are shown whole; beyond this they become too thin
/// a strip across it.
pub const MAX_ASPECT: f32 = 3.0;
/// Above this share of mid-grey pixels in the band, a picture looks muddy on the
/// panel: dithered mid-greys read as noise, and yellowed silk or varnish turns into
/// a flat grey.
pub const MAX_MIDTONES: f32 = 0.6;
/// Dark paintings are lifted toward this mean brightness (0..1).
const SHADOW_TARGET: f32 = 0.42;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArtMeta {
    pub id: String,
    /// Image file next to `art.json`; only used by the built-in collection.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Shown on the clock: Chinese when known.
    pub title: String,
    pub artist: String,
    pub year: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_original: Option<String>,
    /// Full credit, e.g. "Claude Monet (French, 1840–1926)".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_original: Option<String>,
    /// One line about the artist in Chinese, e.g. "后印象派荷兰艺术家（1853－1890）".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist_about: Option<String>,
    /// Chinese genre and movement, e.g. "风景画" and "后印象派".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<String>,
    /// Chinese when the medium is a known one, otherwise the museum's wording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<String>,
    /// What the picture shows, as Chinese labels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub museum: Option<String>,
    /// Chinese narration for the guide pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro: Option<String>,
    /// Heading over `intro`, "讲解" when absent; e.g. "关于画家" when the text is about the artist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro_heading: Option<String>,
    /// Where `intro` was taken from, shown under it, e.g. "摘自维基百科「神奈川冲浪里」条目，CC BY-SA 4.0".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intro_source: Option<String>,
    /// Where to keep the crop, 0..1 on each axis; found automatically when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<[f32; 2]>,
    /// Show the whole picture on white even when it is narrower than the band.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub whole: bool,
    /// Origin and licence, e.g. "Wikimedia Commons，公有领域；Wikidata Q252485".
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

pub struct Artwork {
    pub meta: ArtMeta,
    /// The picture as it appears on the clock: exactly the art band's size.
    pub band: GrayImage,
    /// The whole picture, toned, at most twice the screen size, for pages that show it uncropped.
    pub full: GrayImage,
}

/// The built-in pictures listed in `art.json`, as loaded.
pub fn load_builtins(dir: &Path) -> Result<Vec<(ArtMeta, GrayImage)>, Box<dyn std::error::Error>> {
    let list_path = dir.join("art.json");
    let list: Vec<ArtMeta> = serde_json::from_slice(&std::fs::read(&list_path)?)
        .map_err(|e| format!("{}: {e}", list_path.display()))?;
    if list.is_empty() {
        return Err(format!("{} 里没有画作", list_path.display()).into());
    }
    list.into_iter()
        .map(|meta| {
            let path = dir.join(&meta.file);
            let img = image::open(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .to_luma8();
            Ok((meta, img))
        })
        .collect()
}

/// Tones the picture for the panel and builds both the band and the full view.
pub fn prepare(meta: ArtMeta, img: GrayImage, w: u32, h: u32) -> Artwork {
    let img = tone(img);
    let band = band(&img, w, h, meta.focus, meta.whole);
    let full = if img.width() > 2 * W || img.height() > 2 * H {
        fit(&img, 2 * W, 2 * H)
    } else {
        img
    };
    Artwork { meta, band, full }
}

pub fn tone(img: GrayImage) -> GrayImage {
    lift_shadows(autocontrast(img, 0.01))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// Crops well whatever it shows.
    Wide,
    /// Crops well only when it shows scenery.
    Upright,
}

/// How a picture with this width-over-height crops to the band, or why it cannot.
pub fn shape(aspect: f32) -> Result<Shape, String> {
    if aspect > MAX_ASPECT {
        Err(format!("太宽（宽高比 {aspect:.2}），整幅放进横幅只剩窄窄一条"))
    } else if aspect >= MIN_WIDE_ASPECT {
        Ok(Shape::Wide)
    } else if aspect >= MIN_UPRIGHT_ASPECT {
        Ok(Shape::Upright)
    } else {
        Err(format!("太窄（宽高比 {aspect:.2}），裁成横幅只剩一小段"))
    }
}

/// What the checks found about a picture that passed them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    pub shape: Shape,
    /// Where the band is cropped; `None` when the picture is shown whole.
    pub focus: Option<[f32; 2]>,
    /// Share of mid-grey pixels in what the band shows; lower reads crisper on the panel.
    pub midtones: f32,
}

/// Rejects pictures that crop badly or lose their contrast in grey. The checks look
/// at the picture as the `w`×`h` band shows it, leaving out any white around it.
pub fn check_quality(img: &GrayImage, focus: Option<[f32; 2]>, w: u32, h: u32) -> Result<Fit, String> {
    let (iw, ih) = img.dimensions();
    if iw.min(ih) < 300 {
        return Err(format!("图片太小（{iw}×{ih}）"));
    }
    let shape = shape(iw as f32 / ih as f32)?;
    let (look, focus) = look(img, focus, false, w, h);
    if look.spread < 40.0 {
        return Err(format!("转灰后层次太少（标准差 {:.0}）", look.spread));
    }
    Ok(Fit { shape, focus, midtones: look.midtones })
}

/// How a picture will read on the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    /// Share of mid-grey pixels; above `MAX_MIDTONES` the picture looks muddy.
    pub midtones: f32,
    /// Standard deviation of grey levels; below 40 there is little to see.
    pub spread: f32,
}

/// Measures the picture as the `w`×`h` band shows it, leaving out any white around
/// it, and returns the focus of the crop if there is one.
pub fn look(img: &GrayImage, focus: Option<[f32; 2]>, whole: bool, w: u32, h: u32) -> (Look, Option<[f32; 2]>) {
    let toned = tone(img.clone());
    let (shown, focus) = view(&toned, w, h, focus, whole);
    (Look { midtones: midtones(&shown), spread: std_dev(&shown) }, focus)
}

/// Whether a `sw`×`sh` picture is at least as wide as the `w`×`h` band, and so is
/// shown whole rather than cropped.
pub fn shows_whole(sw: u32, sh: u32, w: u32, h: u32) -> bool {
    sw as u64 * h as u64 >= sh as u64 * w as u64
}

/// What the band shows of a toned picture: all of it when it is wide enough or
/// `whole` asks for it; otherwise a crop at `focus`, or where the detail is. Also
/// returns the focus of a crop.
fn view(img: &GrayImage, w: u32, h: u32, focus: Option<[f32; 2]>, whole: bool) -> (GrayImage, Option<[f32; 2]>) {
    if whole || shows_whole(img.width(), img.height(), w, h) {
        return (fit(img, w, h), None);
    }
    let focus = focus.unwrap_or_else(|| find_focus(img, w, h));
    (cover(img, w, h, focus), Some(focus))
}

/// The `w`×`h` band for a toned picture. A picture shown whole rests on the
/// bottom edge, where the divider runs, centred on white.
pub fn band(img: &GrayImage, w: u32, h: u32, focus: Option<[f32; 2]>, whole: bool) -> GrayImage {
    let (shown, _) = view(img, w, h, focus, whole);
    if shown.dimensions() == (w, h) {
        return shown;
    }
    let mut out = GrayImage::from_pixel(w, h, Luma([255]));
    let x = (w - shown.width()) / 2;
    imageops::replace(&mut out, &shown, x as i64, (h - shown.height()) as i64);
    out
}

/// Rejects pictures that look muddy on the panel. The share shifts by a few points
/// with the size of the copy measured, so it is judged once, when the catalogue is
/// built, rather than again on every download.
pub fn check_midtones(fit: &Fit) -> Result<(), String> {
    if fit.midtones > MAX_MIDTONES {
        return Err(format!("中间灰占 {:.0}%，上屏发灰", fit.midtones * 100.0));
    }
    Ok(())
}

/// Share of pixels between a quarter and three quarters of full brightness.
pub fn midtones(img: &GrayImage) -> f32 {
    let n = (img.width() * img.height()).max(1);
    img.pixels().filter(|p| (64..192).contains(&p.0[0])).count() as f32 / n as f32
}

fn std_dev(img: &GrayImage) -> f32 {
    let n = (img.width() * img.height()) as f64;
    let mean = img.pixels().map(|p| p.0[0] as f64).sum::<f64>() / n;
    let var = img.pixels().map(|p| (p.0[0] as f64 - mean).powi(2)).sum::<f64>() / n;
    var.sqrt() as f32
}

/// Stretches levels so the darkest and lightest `cutoff` share of pixels clip.
/// Museum scans keep the paper tone, which otherwise reads as a dull grey on e-ink.
pub fn autocontrast(mut img: GrayImage, cutoff: f32) -> GrayImage {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let cut = (total as f32 * cutoff) as u64;
    let level_at = |order: &mut dyn Iterator<Item = usize>| {
        let mut seen = 0u64;
        for v in order {
            seen += hist[v];
            if seen > cut {
                return v;
            }
        }
        0
    };
    let lo = level_at(&mut (0..256));
    let hi = level_at(&mut (0..256).rev());
    if hi <= lo {
        return img;
    }
    let scale = 255.0 / (hi - lo) as f32;
    for p in img.pixels_mut() {
        p.0[0] = ((p.0[0] as f32 - lo as f32) * scale).clamp(0.0, 255.0).round() as u8;
    }
    img
}

/// Brightens dark paintings with a gamma curve: on the panel, dithered shadows
/// merge into one flat black.
fn lift_shadows(mut img: GrayImage) -> GrayImage {
    let n = (img.width() * img.height()).max(1) as f64;
    let mean = (img.pixels().map(|p| p.0[0] as f64).sum::<f64>() / n / 255.0) as f32;
    if mean >= SHADOW_TARGET || mean <= 0.02 {
        return img;
    }
    let gamma = (SHADOW_TARGET.ln() / mean.ln()).clamp(0.55, 1.0);
    let lut: Vec<u8> = (0..256)
        .map(|v| (255.0 * (v as f32 / 255.0).powf(gamma)).round() as u8)
        .collect();
    for p in img.pixels_mut() {
        p.0[0] = lut[p.0[0] as usize];
    }
    img
}

/// Crops to the target aspect around `focus`, then scales to exactly `w`×`h`.
pub fn cover(img: &GrayImage, w: u32, h: u32, focus: [f32; 2]) -> GrayImage {
    let (sw, sh) = img.dimensions();
    let target = w as f32 / h as f32;
    let (cw, ch) = if sw as f32 / sh as f32 > target {
        (((sh as f32 * target).round() as u32).min(sw), sh)
    } else {
        (sw, ((sw as f32 / target).round() as u32).min(sh))
    };
    let x = ((sw - cw) as f32 * focus[0].clamp(0.0, 1.0)).round() as u32;
    let y = ((sh - ch) as f32 * focus[1].clamp(0.0, 1.0)).round() as u32;
    let crop = imageops::crop_imm(img, x, y, cw, ch).to_image();
    imageops::resize(&crop, w, h, FilterType::Lanczos3)
}

/// Picks the crop window with the most detail, measured as local contrast, with a
/// mild pull toward the centre so that plain sky or a border loses to the subject.
pub fn find_focus(img: &GrayImage, w: u32, h: u32) -> [f32; 2] {
    let (sw, sh) = img.dimensions();
    let s = (w as f32 / sw as f32).max(h as f32 / sh as f32);
    let cw = ((sw as f32 * s).round() as u32).max(w);
    let ch = ((sh as f32 * s).round() as u32).max(h);
    let vertical = ch > h;
    let (len, win) = if vertical { (ch, h) } else { (cw, w) };
    if len <= win {
        return [0.5, 0.5];
    }
    let small = imageops::resize(img, cw, ch, FilterType::Triangle);
    let mut energy = vec![0f32; len as usize];
    for y in 0..ch - 1 {
        for x in 0..cw - 1 {
            let p = small.get_pixel(x, y).0[0] as f32;
            let e = (p - small.get_pixel(x + 1, y).0[0] as f32).abs()
                + (p - small.get_pixel(x, y + 1).0[0] as f32).abs();
            energy[if vertical { y } else { x } as usize] += e;
        }
    }
    let span = len - win;
    let mut sum: f32 = energy[..win as usize].iter().sum();
    let mut best = (f32::MIN, 0u32);
    for off in 0..=span {
        if off > 0 {
            sum += energy[(off + win - 1) as usize] - energy[(off - 1) as usize];
        }
        let t = off as f32 / span as f32 - 0.5;
        let score = sum * (1.0 - 0.6 * t * t);
        if score > best.0 {
            best = (score, off);
        }
    }
    let f = best.1 as f32 / span as f32;
    if vertical {
        [0.5, f]
    } else {
        [f, 0.5]
    }
}

/// The size of a `sw`×`sh` picture scaled to fit inside `w`×`h`.
pub fn fit_size(sw: u32, sh: u32, w: u32, h: u32) -> (u32, u32) {
    let s = (w as f32 / sw as f32).min(h as f32 / sh as f32);
    (
        ((sw as f32 * s).round() as u32).clamp(1, w),
        ((sh as f32 * s).round() as u32).clamp(1, h),
    )
}

/// Scales to fit inside `w`×`h`, keeping the aspect ratio.
pub fn fit(img: &GrayImage, w: u32, h: u32) -> GrayImage {
    let (fw, fh) = fit_size(img.width(), img.height(), w, h);
    imageops::resize(img, fw, fh, FilterType::Lanczos3)
}


/// 1 px black rectangle outline.
pub fn frame(img: &mut GrayImage, x: u32, y: u32, w: u32, h: u32) {
    let (iw, ih) = img.dimensions();
    for xx in x..(x + w).min(iw) {
        for yy in [y, y + h - 1] {
            if yy < ih {
                img.put_pixel(xx, yy, Luma([0]));
            }
        }
    }
    for yy in y..(y + h).min(ih) {
        for xx in [x, x + w - 1] {
            if xx < iw {
                img.put_pixel(xx, yy, Luma([0]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat grey with a band of fine stripes in rows `y0..y0 + 60`.
    fn striped(w: u32, h: u32, y0: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, y| {
            let busy = (y0..y0 + 60).contains(&y) && (x / 3) % 2 == 0;
            Luma([if busy { 20 } else { 200 }])
        })
    }

    #[test]
    fn focus_follows_the_detail() {
        let top = find_focus(&striped(800, 600, 40), 400, 204);
        let bottom = find_focus(&striped(800, 600, 500), 400, 204);
        assert!(top[1] < 0.3, "{top:?}");
        assert!(bottom[1] > 0.7, "{bottom:?}");
    }

    #[test]
    fn narrower_pictures_are_cropped_to_fill_the_band() {
        let meta = ArtMeta::default();
        for (w, h) in [(450, 500), (900, 500)] {
            let art = prepare(meta.clone(), striped(w, h, 100), 400, 204);
            assert_eq!(art.band.dimensions(), (400, 204));
            let edge = (0..204).flat_map(|y| (0..4).map(move |x| (x, y)));
            let darkest = edge.map(|(x, y)| art.band.get_pixel(x, y).0[0]).min();
            assert!(darkest < Some(128), "picture reaches the edge, no white mat");
            assert!(art.full.width() <= 2 * W && art.full.height() <= 2 * H);
        }
    }

    #[test]
    fn wider_pictures_are_shown_whole_on_the_bottom_edge() {
        // Dark columns at both ends, so a crop would lose one of them.
        let wide = GrayImage::from_fn(1000, 400, |x, _| Luma([if !(20..980).contains(&x) || (x / 3) % 2 == 0 { 20 } else { 200 }]));
        let art = prepare(ArtMeta { focus: Some([0.0, 0.5]), ..ArtMeta::default() }, wide.clone(), 400, 204);
        assert_eq!(art.band.dimensions(), (400, 204));
        let row_is_white = |y: u32| (0..400).all(|x| art.band.get_pixel(x, y).0[0] == 255);
        assert!((0..42).all(row_is_white), "white above the picture");
        assert!(!(46..204).any(row_is_white), "picture down to the bottom edge");
        for x in [2, 397] {
            assert!(art.band.get_pixel(x, 120).0[0] < 64, "both ends shown, column {x}");
        }
        let fit = check_quality(&wide, None, 400, 204).unwrap();
        assert_eq!(fit.focus, None);
        assert!(fit.midtones < 0.5, "margin is not measured: {}", fit.midtones);
    }

    #[test]
    fn narrow_pictures_can_be_shown_whole() {
        let tall = GrayImage::from_fn(300, 600, |x, _| Luma([if (x / 3) % 2 == 0 { 20 } else { 200 }]));
        let art = prepare(ArtMeta { whole: true, ..ArtMeta::default() }, tall, 400, 204);
        assert_eq!(art.band.dimensions(), (400, 204));
        let column_is_white = |x: u32| (0..204).all(|y| art.band.get_pixel(x, y).0[0] == 255);
        assert!(column_is_white(0) && column_is_white(399), "white on both sides");
        assert!(!column_is_white(200), "picture in the middle");
    }

    #[test]
    fn poor_pictures_are_rejected() {
        let check = |img: &GrayImage| check_quality(img, None, 400, 204).map(|f| f.shape);
        let flat = GrayImage::from_pixel(800, 600, Luma([128]));
        assert!(check(&flat).is_err());
        assert_eq!(check(&striped(800, 600, 0)), Ok(Shape::Wide));
        assert_eq!(check(&striped(600, 700, 0)), Ok(Shape::Upright));
        assert!(check(&striped(200, 600, 0)).is_err(), "too small");
        assert!(check(&striped(400, 600, 0)).is_err(), "too narrow");
        assert!(check(&striped(1800, 500, 0)).is_err(), "too wide");
    }

    #[test]
    fn muddy_pictures_are_rejected() {
        // Like faded ink on yellowed silk: a few true darks and lights, the rest mid-grey.
        let muddy = GrayImage::from_fn(800, 500, |x, y| {
            Luma([match (x / 8 * 7 + y / 8 * 13) % 100 {
                0..=7 => 0,
                8..=15 => 255,
                n => 100 + (n as u8 % 50),
            }])
        });
        let fit = check_quality(&muddy, None, 400, 204).unwrap();
        let err = check_midtones(&fit).unwrap_err();
        assert!(err.contains("中间灰"), "{err}");
        assert!(check_midtones(&check_quality(&striped(800, 600, 0), None, 400, 204).unwrap()).is_ok());
    }

    #[test]
    fn dark_pictures_get_lighter() {
        let dark = GrayImage::from_fn(100, 100, |x, _| Luma([if x < 90 { 10 } else { 255 }]));
        let mean = |i: &GrayImage| i.pixels().map(|p| p.0[0] as u32).sum::<u32>() / 10_000;
        assert!(mean(&lift_shadows(dark.clone())) > mean(&dark));
    }
}
