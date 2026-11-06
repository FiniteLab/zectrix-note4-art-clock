//! Layout C: artwork across the top, a white band below with a large clock; and the
//! guide pages that open with the Confirm key.
//!
//! Minimum legible sizes on the ~119 PPI panel, checked on the device:
//! sans 14 px, serif 16 px, the pixel font only at its native 12 px without antialiasing.
use crate::{
    art::{self, ArtMeta, Artwork},
    icons::{self, BATTERY_W, BOLT_W, WIFI_W},
    render::{Canvas, Face, Text, H, W},
};
use chrono::{DateTime, Datelike, TimeZone, Timelike};
use image::GrayImage;
use std::path::Path;

pub const BAND: u32 = 96;
pub const ART_H: u32 = H - BAND;
pub const TIME_PX: f32 = 92.0;
pub const TIME_X: f32 = 14.0;
pub const INFO_X: u32 = 270;
const DIVIDER_X: u32 = INFO_X - 12;
const WEEKDAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];

pub const GUIDE_HINT: &str = "上下键翻页 · 确认键返回";
const MARGIN: f32 = 16.0;
/// Text pages: body between these, under a title rule and above the footer.
const PAGE_TOP: f32 = 40.0;
const PAGE_BOTTOM: f32 = 276.0;
const FOOTER_BASELINE: f32 = 293.0;
const LABEL_W: f32 = 44.0;

pub struct Fonts {
    pub numerals: Face,
    pub sans: Face,
    pub sans_bold: Face,
    pub pixel: Face,
}

impl Fonts {
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        Ok(Self {
            numerals: Face::load(&dir.join("GFSDidot-Regular.ttf"))?,
            sans: Face::load(&dir.join("SourceHanSansCN-Regular.otf"))?,
            sans_bold: Face::load(&dir.join("SourceHanSansCN-Bold.otf"))?,
            pixel: Face::load(&dir.join("FusionPixel-12px-zh_hans.otf"))?,
        })
    }
}

/// Device telemetry shown in the status chip; `None` fields are left out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Status {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub wifi_rssi_dbm: Option<i8>,
}

pub fn clock_c<Tz: TimeZone>(
    fonts: &Fonts,
    art: &Artwork,
    t: &DateTime<Tz>,
    status: Option<&Status>,
) -> Canvas {
    let mut c = Canvas::new(255);
    c.paste(&art.band, 0, 0);
    c.hline(0, W - 1, ART_H, 0);
    if let Some(status) = status {
        status_chip(&mut c, fonts, status);
    }

    let time = format!("{:02}:{:02}", t.hour(), t.minute());
    c.text(Text::smooth(&fonts.numerals, TIME_PX), TIME_X, 284.0, &time, 0);
    c.vline(DIVIDER_X, ART_H + 14, H - 12, 0);

    let x = INFO_X as f32;
    let max_w = (W - INFO_X - 4) as f32;
    let weekday = WEEKDAYS[t.weekday().num_days_from_monday() as usize];
    let date = format!("{}月{}日", t.month(), t.day());
    c.text_fit(Text::smooth(&fonts.sans_bold, 18.0), x, 228.0, weekday, max_w, 0);
    c.text_fit(Text::smooth(&fonts.sans, 16.0), x, 251.0, &date, max_w, 0);
    let title_style = Text::smooth(&fonts.sans, 14.0);
    let title = short_title(&art.meta.title);
    let title = match title.split_once([':', '：']) {
        Some((head, _)) if title_style.width(&title) > max_w && !head.trim().is_empty() => head.trim().to_string(),
        _ => title,
    };
    c.text_fit(title_style, x, 272.0, &title, max_w, 0);
    let pixel = Text::crisp(&fonts.pixel, 12.0);
    let credit = credit(pixel, &art.meta.artist, &art.meta.year, max_w);
    c.text(pixel, x, 290.0, &credit, 0);
    c
}

/// The title without bracketed asides, which Wikidata labels often carry:
/// "伊丽莎白·法伦（生于约1759年，卒于1829年），即后来的德比伯爵夫人" →
/// "伊丽莎白·法伦，即后来的德比伯爵夫人".
pub fn short_title(title: &str) -> String {
    let mut out = String::new();
    let mut depth = 0u32;
    for c in title.chars() {
        match c {
            '(' | '（' => depth += 1,
            ')' | '）' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out = out.replace(" ,", ",").replace("，，", "，");
    let out = out.trim_matches(|c: char| c == '，' || c == ',' || c.is_whitespace());
    if out.is_empty() {
        title.trim().to_string()
    } else {
        out.to_string()
    }
}

fn join_credit(artist: &str, year: &str) -> String {
    match (artist.is_empty(), year.is_empty()) {
        (false, false) => format!("{artist} · {year}"),
        (false, true) => artist.to_string(),
        _ => year.to_string(),
    }
}

/// "文森特·梵高 · 1889", dropping given names ("梵高 · 1889") and then truncating
/// until it fits, so the year survives a long name.
pub fn credit(style: Text, artist: &str, year: &str, max_w: f32) -> String {
    let mut name = artist.trim();
    loop {
        let s = join_credit(name, year);
        if style.width(&s) <= max_w {
            return s;
        }
        match name.find(['·', ' ']) {
            Some(i) => name = name[i..].trim_start_matches(['·', ' ']),
            None => return style.fit(&s, max_w),
        }
    }
}

/// White chip in the art's top-right corner: Wi-Fi arcs, battery and percentage.
fn status_chip(c: &mut Canvas, fonts: &Fonts, s: &Status) {
    const PAD: i32 = 3;
    const Y: i32 = 4;
    const CHIP_H: i32 = 16;
    let pixel = Text::crisp(&fonts.pixel, 12.0);
    let percent = s.battery_percent.map(|p| format!("{p}%"));
    let mut w = PAD * 2;
    if s.wifi_rssi_dbm.is_some() {
        w += WIFI_W;
    }
    if let Some(text) = &percent {
        if s.wifi_rssi_dbm.is_some() {
            w += 5;
        }
        if s.charging {
            w += BOLT_W + 2;
        }
        w += BATTERY_W + 3 + pixel.width(text).ceil() as i32;
    }
    if w == PAD * 2 {
        return;
    }
    let x0 = W as i32 - 4 - w;
    icons::fill_rounded(c, x0, Y, w, CHIP_H, 255);
    let mut x = x0 + PAD;
    if let Some(rssi) = s.wifi_rssi_dbm {
        icons::wifi(c, x, Y + 2, icons::wifi_level(rssi));
        x += WIFI_W + 5;
    }
    if let (Some(p), Some(text)) = (s.battery_percent, &percent) {
        if s.charging {
            icons::bolt(c, x, Y + 4);
            x += BOLT_W + 2;
        }
        icons::battery(c, x, Y + 3, p);
        x += BATTERY_W + 3;
        c.text(pixel, x as f32, (Y + 12) as f32, text, 0);
    }
}

/// The guide: the whole picture first, then the story, the facts and the museum's text.
pub fn guide(fonts: &Fonts, art: &Artwork) -> Vec<Canvas> {
    let mut pages = vec![cover(fonts, art)];
    let bold = Text::smooth(&fonts.sans_bold, 16.0);
    for runs in guide_text(fonts, &art.meta) {
        let mut c = Canvas::new(255);
        let title = short_title(&art.meta.title);
        c.text_fit(bold, MARGIN, 24.0, &title, W as f32 - 2.0 * MARGIN, 0);
        c.hline(MARGIN as u32, W - MARGIN as u32 - 1, 32, 0);
        for r in runs {
            c.text(r.style, r.x, r.baseline, &r.text, 0);
        }
        pages.push(c);
    }
    let total = pages.len();
    let pixel = Text::crisp(&fonts.pixel, 12.0);
    let hint_x = W as f32 - MARGIN - pixel.width(GUIDE_HINT);
    for (i, c) in pages.iter_mut().enumerate() {
        c.text(pixel, MARGIN, FOOTER_BASELINE, &format!("{}/{total}", i + 1), 0);
        c.text(pixel, hint_x, FOOTER_BASELINE, GUIDE_HINT, 0);
    }
    pages
}

/// The uncropped picture, beside or above its caption, whichever shows it larger.
fn cover(fonts: &Fonts, art: &Artwork) -> Canvas {
    const TOP: u32 = 10;
    const WIDE_H: u32 = 222;
    const TALL_W: u32 = 228;
    const TALL_H: u32 = 262;
    let mut c = Canvas::new(255);
    let m = &art.meta;
    let title = Text::smooth(&fonts.sans_bold, 18.0);
    let small = Text::smooth(&fonts.sans, 14.0);
    let (sw, sh) = art.full.dimensions();
    let wide = art::fit_size(sw, sh, W - 2 * MARGIN as u32, WIDE_H);
    let tall = art::fit_size(sw, sh, TALL_W, TALL_H);
    if wide.0 * wide.1 >= tall.0 * tall.1 {
        let pic = art::fit(&art.full, wide.0, wide.1);
        place(&mut c, &pic, (W - pic.width()) / 2, TOP + (WIDE_H - pic.height()) / 2);
        let max_w = W as f32 - 2.0 * MARGIN;
        c.text_fit(title, MARGIN, 252.0, &m.title, max_w, 0);
        c.text_fit(small, MARGIN, 271.0, &join_credit(&m.artist, &m.year), max_w, 0);
        return c;
    }
    let pic = art::fit(&art.full, tall.0, tall.1);
    let left = MARGIN as u32;
    place(&mut c, &pic, left + (TALL_W - pic.width()) / 2, TOP + (TALL_H - pic.height()) / 2);
    let x = (left + TALL_W + 14) as f32;
    let w = W as f32 - x - 10.0;
    let mut baseline = 36.0;
    let mut lines = title.wrap(&m.title, w);
    if lines.len() > 4 {
        lines.truncate(4);
        lines[3] = title.fit(&format!("{}…", lines[3]), w);
    }
    for line in &lines {
        c.text(title, x, baseline, line, 0);
        baseline += 25.0;
    }
    baseline += 8.0;
    let details = [Some(m.artist.as_str()), Some(m.year.as_str()), m.museum.as_deref()];
    for detail in details.into_iter().flatten().filter(|d| !d.is_empty()) {
        for line in small.wrap(detail, w).into_iter().take(3) {
            c.text(small, x, baseline, &line, 0);
            baseline += 20.0;
        }
    }
    c
}

fn place(c: &mut Canvas, pic: &GrayImage, x: u32, y: u32) {
    c.paste(pic, x as i64, y as i64);
    art::frame(&mut c.0, x - 1, y - 1, pic.width() + 2, pic.height() + 2);
}

struct Run<'a> {
    style: Text<'a>,
    x: f32,
    baseline: f32,
    text: String,
}

/// Places lines top to bottom and starts a new page when one is full.
struct Flow<'a> {
    pages: Vec<Vec<Run<'a>>>,
    y: f32,
}

impl<'a> Flow<'a> {
    fn new() -> Self {
        Self { pages: vec![Vec::new()], y: PAGE_TOP }
    }

    fn at_top(&self) -> bool {
        self.y <= PAGE_TOP
    }

    /// Starts a new page unless `h` more pixels fit on this one.
    fn reserve(&mut self, h: f32) {
        if self.y + h > PAGE_BOTTOM && !self.at_top() {
            self.pages.push(Vec::new());
            self.y = PAGE_TOP;
        }
    }

    fn gap(&mut self, h: f32) {
        if !self.at_top() {
            self.y += h;
        }
    }

    /// Adds one line of height `advance` and returns its baseline.
    fn line(&mut self, style: Text<'a>, x: f32, text: String, advance: f32) -> f32 {
        self.reserve(advance);
        // Centres the em box (about 0.88 above and 0.12 below the baseline) in the line.
        let baseline = self.y + advance / 2.0 + style.px * 0.38;
        self.push(Run { style, x, baseline, text });
        self.y += advance;
        baseline
    }

    fn push(&mut self, run: Run<'a>) {
        self.pages.last_mut().expect("a flow always has a page").push(run);
    }

    fn finish(self) -> Vec<Vec<Run<'a>>> {
        self.pages.into_iter().filter(|p| !p.is_empty()).collect()
    }
}

fn guide_text<'a>(fonts: &'a Fonts, m: &ArtMeta) -> Vec<Vec<Run<'a>>> {
    let body = Text::smooth(&fonts.sans, 16.0);
    let bold = Text::smooth(&fonts.sans_bold, 16.0);
    let pixel = Text::crisp(&fonts.pixel, 12.0);
    let mut f = Flow::new();
    if let Some(intro) = nonempty(&m.intro) {
        heading(&mut f, bold, nonempty(&m.intro_heading).unwrap_or("讲解"));
        paragraphs(&mut f, body, intro, 24.0, 6.0);
        if let Some(source) = nonempty(&m.intro_source) {
            f.gap(4.0);
            for line in pixel.wrap(source, W as f32 - 2.0 * MARGIN) {
                f.line(pixel, MARGIN, line, 16.0);
            }
        }
    }
    let rows = facts(m);
    if !rows.is_empty() {
        f.gap(12.0);
        heading(&mut f, bold, "作品信息");
        for (label, value) in &rows {
            row(&mut f, bold, body, label, value);
        }
    }
    if !m.source.is_empty() {
        f.gap(10.0);
        let source = format!("图片与资料：{}", m.source);
        for line in pixel.wrap(&source, W as f32 - 2.0 * MARGIN) {
            f.line(pixel, MARGIN, line, 16.0);
        }
    }
    f.finish()
}

fn nonempty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Label–value pairs for the facts section, in Chinese where the data allows.
fn facts(m: &ArtMeta) -> Vec<(&'static str, String)> {
    let mut rows = Vec::new();
    if let Some(original) = nonempty(&m.title_original).filter(|t| *t != m.title) {
        rows.push(("原名", original.to_string()));
    }
    let original = nonempty(&m.artist_original);
    let mut artist: Vec<&str> = Vec::new();
    // "Édouard Manet (French, 1832–1883)" already names "Édouard Manet".
    if !original.is_some_and(|o| o.starts_with(m.artist.as_str())) {
        artist.push(&m.artist);
    }
    artist.extend(nonempty(&m.artist_about));
    artist.extend(original);
    artist.retain(|a| !a.is_empty());
    if !artist.is_empty() {
        rows.push(("作者", artist.join("\n")));
    }
    if !m.year.is_empty() {
        rows.push(("年代", m.year.clone()));
    }
    for (label, value) in [
        ("类型", &m.genre),
        ("流派", &m.movement),
        ("材质", &m.medium),
        ("尺寸", &m.dimensions),
    ] {
        if let Some(v) = nonempty(value) {
            rows.push((label, v.to_string()));
        }
    }
    if !m.subjects.is_empty() {
        rows.push(("画中", m.subjects.join("、")));
    }
    if let Some(museum) = nonempty(&m.museum) {
        rows.push(("馆藏", museum.to_string()));
    }
    rows
}

fn heading<'a>(f: &mut Flow<'a>, style: Text<'a>, text: &str) {
    // Keeps the heading on the same page as the first two lines under it.
    f.reserve(26.0 + 2.0 * 24.0);
    f.line(style, MARGIN, text.to_string(), 26.0);
}

fn paragraphs<'a>(f: &mut Flow<'a>, style: Text<'a>, text: &str, advance: f32, para_gap: f32) {
    let width = W as f32 - 2.0 * MARGIN;
    for (i, para) in text.split('\n').map(str::trim).filter(|p| !p.is_empty()).enumerate() {
        if i > 0 {
            f.gap(para_gap);
        }
        for line in style.wrap(para, width) {
            f.line(style, MARGIN, line, advance);
        }
    }
}

fn row<'a>(f: &mut Flow<'a>, label_style: Text<'a>, value_style: Text<'a>, label: &str, value: &str) {
    const ADVANCE: f32 = 23.0;
    let x = MARGIN + LABEL_W;
    let width = W as f32 - MARGIN - x;
    let lines: Vec<String> = value
        .split('\n')
        .filter(|v| !v.trim().is_empty())
        .flat_map(|v| value_style.wrap(v.trim(), width))
        .collect();
    f.reserve(ADVANCE * lines.len().min(3) as f32);
    for (i, line) in lines.into_iter().enumerate() {
        let baseline = f.line(value_style, x, line, ADVANCE);
        if i == 0 {
            f.push(Run { style: label_style, x: MARGIN, baseline, text: label.to_string() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{load_builtins, prepare, Artwork};

    fn load_collection(dir: &Path, w: u32, h: u32) -> Vec<Artwork> {
        load_builtins(dir).unwrap().into_iter().map(|(meta, img)| prepare(meta, img, w, h)).collect()
    }
    use chrono::{Local, NaiveDate};

    fn assets() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets")
    }

    #[test]
    fn every_time_of_day_stays_left_of_the_divider() {
        let fonts = Fonts::load(&assets().join("fonts")).unwrap();
        let widest = (0..24 * 60)
            .map(|m| fonts.numerals.width(TIME_PX, &format!("{:02}:{:02}", m / 60, m % 60)))
            .fold(0.0f32, f32::max);
        assert!(TIME_X + widest < DIVIDER_X as f32 - 4.0, "widest time is {widest}px");
    }

    #[test]
    fn long_titles_are_shortened_to_fit() {
        let fonts = Fonts::load(&assets().join("fonts")).unwrap();
        let style = Text::smooth(&fonts.sans, 14.0);
        let fitted = style.fit("一个非常非常长的画作标题会被截短", 120.0);
        assert!(fitted.ends_with('…'));
        assert!(style.width(&fitted) <= 120.0);
    }

    #[test]
    fn long_names_lose_given_names_before_the_year() {
        let fonts = Fonts::load(&assets().join("fonts")).unwrap();
        let pixel = Text::crisp(&fonts.pixel, 12.0);
        let max_w = (W - INFO_X - 4) as f32;
        assert_eq!(credit(pixel, "古斯塔夫·卡耶博特", "1877", max_w), "卡耶博特 · 1877");
        assert_eq!(credit(pixel, "葛饰北斋", "1830–33", max_w), "葛饰北斋 · 1830–33");
        let latin = credit(pixel, "Jean-Baptiste-Camille Corot", "ca. 1865", max_w);
        assert!(latin.starts_with("Corot") && pixel.width(&latin) <= max_w, "{latin}");
    }

    #[test]
    fn titles_drop_bracketed_asides() {
        assert_eq!(
            short_title("伊丽莎白·法伦（生于约1759年，卒于1829年），即后来的德比伯爵夫人"),
            "伊丽莎白·法伦，即后来的德比伯爵夫人"
        );
        assert_eq!(
            short_title("Krishna Revels with the Gopis: Page from a Dispersed Gita Govinda (Song of the Cowherds)"),
            "Krishna Revels with the Gopis: Page from a Dispersed Gita Govinda"
        );
        assert_eq!(short_title("（无题）"), "（无题）");
        assert_eq!(short_title("月出"), "月出");
    }

    #[test]
    fn renders_every_artwork_with_and_without_status() {
        let fonts = Fonts::load(&assets().join("fonts")).unwrap();
        let arts = load_collection(&assets().join("art"), W, ART_H);
        let t = NaiveDate::from_ymd_opt(2026, 12, 31)
            .unwrap()
            .and_hms_opt(23, 59, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let full = Status { battery_percent: Some(100), charging: true, wifi_rssi_dbm: Some(-72) };
        for art in &arts {
            for status in [None, Some(&full)] {
                let frame = clock_c(&fonts, art, &t, status);
                assert_eq!(frame.0.dimensions(), (W, H));
            }
        }
    }

    #[test]
    fn guide_text_stays_clear_of_the_footer() {
        let fonts = Fonts::load(&assets().join("fonts")).unwrap();
        let mut arts = load_collection(&assets().join("art"), W, ART_H);
        let mut long = arts.pop().unwrap();
        long.meta.intro = long.meta.intro.map(|t| t.repeat(4));
        long.meta.subjects = (0..6).map(|i| format!("主题{i}")).collect();
        long.meta.intro_heading = Some("关于画家".into());
        long.meta.intro_source = Some("摘自维基百科「乔治·英尼斯」条目，CC BY-SA 4.0".into());
        arts.push(long);
        for art in &arts {
            assert!(art.meta.intro.is_some(), "{} needs a Chinese intro", art.meta.id);
            let pages = guide(&fonts, art);
            assert!(pages.len() >= 2, "{}", art.meta.id);
            for (i, page) in pages.iter().enumerate() {
                let gap = PAGE_BOTTOM as u32 + 1..282;
                let ink = page.ink_rows(255);
                assert!(
                    !ink.iter().any(|y| gap.contains(y)),
                    "{} page {} draws into the footer gap",
                    art.meta.id,
                    i + 1
                );
            }
        }
    }
}
