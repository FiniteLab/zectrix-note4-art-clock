//! Rotation: which work fills which slot of time. Catalogue works are cached on
//! disk and topped up in the background; the owner's own pictures take their turns
//! in between, as the settings say.
//!
//! Cache directory:
//!   state.json        which work fills which slot, and works found unusable
//!   works/<id>.json   metadata, works/<id>.jpg greyscale source (≤ 1200 px)
//!   thumbs/<id>.jpg   small copies for the web admin
use crate::{
    art::{self, ArtMeta},
    catalog::Entry,
    layout::ART_H,
    net::{Client, FetchError},
    render::W,
    store::{self, Mode, Settings, Store},
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike};
use image::{codecs::jpeg::JpegEncoder, imageops, GrayImage};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs, io,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

/// Slots of history kept for browsing with the up/down keys, the current one included.
pub const KEEP: usize = 14;
/// Past slots filled in on a fresh cache, so there is something to browse from the start.
pub const BEHIND: i64 = 6;
/// Cached sources are scaled to fit this square; the panel is 400 px wide.
pub const KEEP_SIDE: u32 = 1200;
/// Works in the catalogue passed the checks when it was built, so a failure here
/// usually means the image changed on Commons; a few tries are plenty.
const TRIES_PER_SLOT: usize = 8;
/// Network failures on one work before it is skipped for the rest of this run.
const MAX_FAILURES: u32 = 3;
const MAX_IMAGE: u64 = 32 << 20;
/// `Entry::group` of the famous works; the others fill the slots between them.
pub const FAMOUS: &str = "名作";
const DAY_MINUTES: u32 = 24 * 60;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Which work fills which rotation slot, by the slot's local start time.
    #[serde(default)]
    pub slots: BTreeMap<NaiveDateTime, String>,
    /// Older versions kept one work per day; read once and turned into slots.
    #[serde(default, skip_serializing)]
    days: BTreeMap<NaiveDate, String>,
    #[serde(default)]
    pub rejected: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `$XDG_CACHE_HOME/art-clock`, else `~/.cache/art-clock`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
            .join("art-clock")
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn works(&self) -> PathBuf {
        self.dir.join("works")
    }

    fn paths(&self, key: &str) -> (PathBuf, PathBuf) {
        let works = self.works();
        (works.join(format!("{key}.json")), works.join(format!("{key}.jpg")))
    }

    pub fn state(&self) -> State {
        let mut state: State = read_json(&self.dir.join("state.json")).unwrap_or_default();
        for (day, key) in std::mem::take(&mut state.days) {
            state.slots.entry(day.and_time(NaiveTime::MIN)).or_insert(key);
        }
        state
    }

    fn save_state(&self, state: &State) -> io::Result<()> {
        write_atomic(&self.dir.join("state.json"), &serde_json::to_vec_pretty(state)?)
    }

    pub fn has_work(&self, key: &str) -> bool {
        let (meta, img) = self.paths(key);
        meta.is_file() && img.is_file()
    }

    pub fn load_meta(&self, key: &str) -> Option<ArtMeta> {
        read_json(&self.paths(key).0)
    }

    pub fn load_work(&self, key: &str) -> Result<(ArtMeta, GrayImage), String> {
        let (meta_path, img_path) = self.paths(key);
        let meta: ArtMeta = read_json(&meta_path)
            .ok_or_else(|| format!("{} 读不出来", meta_path.display()))?;
        let img = image::open(&img_path)
            .map_err(|e| format!("{}：{e}", img_path.display()))?
            .to_luma8();
        Ok((meta, img))
    }

    pub fn save_work(&self, meta: &ArtMeta, img: &GrayImage) -> io::Result<()> {
        let (_, img_path) = self.paths(&meta.id);
        let mut jpg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpg, 92)
            .encode_image(img)
            .map_err(io::Error::other)?;
        write_atomic(&img_path, &jpg)?;
        self.save_meta(meta)
    }

    pub fn save_meta(&self, meta: &ArtMeta) -> io::Result<()> {
        write_atomic(&self.paths(&meta.id).0, &serde_json::to_vec_pretty(meta)?)
    }

    pub fn remove_work(&self, key: &str) -> io::Result<()> {
        let (meta, img) = self.paths(key);
        for path in [meta, img] {
            match fs::remove_file(path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Keeps the `keep` most recently saved works and those in `also`, deleting the rest.
    pub fn prune(&self, keep: usize, also: &[&str]) -> io::Result<()> {
        let Ok(entries) = fs::read_dir(self.works()) else {
            return Ok(());
        };
        let mut works: Vec<(std::time::SystemTime, String)> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| {
                let stem = e.path().file_stem()?.to_str()?.to_string();
                Some((e.metadata().ok()?.modified().ok()?, stem))
            })
            .collect();
        works.sort_by_key(|w| std::cmp::Reverse(w.0));
        for (_, key) in works.into_iter().skip(keep) {
            if !also.contains(&key.as_str()) {
                self.remove_work(&key)?;
            }
        }
        Ok(())
    }
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Other threads read these files while they are written.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn per_day(interval: u32) -> u32 {
    DAY_MINUTES.div_ceil(interval.clamp(1, DAY_MINUTES))
}

/// Start of the `interval`-minute slot holding `t`; each day's slots start at midnight.
pub fn slot_start(t: NaiveDateTime, interval: u32) -> NaiveDateTime {
    let interval = interval.clamp(1, DAY_MINUTES);
    let minutes = t.hour() * 60 + t.minute();
    let start = minutes - minutes % interval;
    let time = NaiveTime::from_hms_opt(start / 60, start % 60, 0).expect("a minute of the day");
    t.date().and_time(time)
}

pub fn next_slot(start: NaiveDateTime, interval: u32) -> NaiveDateTime {
    let next = start + TimeDelta::minutes(interval.clamp(1, DAY_MINUTES) as i64);
    if next.date() != start.date() {
        next.date().and_time(NaiveTime::MIN)
    } else {
        next
    }
}

pub fn previous_slot(start: NaiveDateTime, interval: u32) -> NaiveDateTime {
    slot_start(start - TimeDelta::minutes(1), interval)
}

/// Where a slot falls in the endless rotation, counting slots from the first day of
/// the common era; with daily slots this is the day number.
pub fn slot_number(start: NaiveDateTime, interval: u32) -> i64 {
    let interval = interval.clamp(1, DAY_MINUTES);
    let day = start.date().num_days_from_ce() as i64;
    let minutes = start.hour() * 60 + start.minute();
    day * per_day(interval) as i64 + (minutes / interval) as i64
}

/// Which source fills a slot, and how far along that source's own sequence it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Catalog(i64),
    Gallery(i64),
}

/// Every `c + g` slots hold `c` catalogue works and `g` own pictures, spread evenly.
pub fn pick(n: i64, c: u32, g: u32) -> Pick {
    match (c, g) {
        (_, 0) => Pick::Catalog(n),
        (0, _) => Pick::Gallery(n),
        _ => {
            let (round, g) = ((c + g) as i64, g as i64);
            let before = (n * g).div_euclid(round);
            if ((n + 1) * g).div_euclid(round) > before {
                Pick::Gallery(before)
            } else {
                Pick::Catalog(n - before)
            }
        }
    }
}

/// Work keys in up/down order: the one to show at `now` (the fixed picture in fixed
/// mode; otherwise the slot's, or the latest earlier one while the slot's is still
/// on its way), then earlier slots, newest first. Callers drop keys they cannot load.
pub fn lineup(state: &State, settings: &Settings, now: NaiveDateTime) -> Vec<String> {
    let mut out: Vec<String> = settings.fixed_key().map(str::to_string).into_iter().collect();
    let current = slot_start(now, settings.interval);
    for key in state.slots.range(..=current).rev().map(|(_, k)| k) {
        if !out.contains(key) {
            out.push(key.clone());
        }
    }
    out
}

/// "11月6日" for daily slots, "11月6日 14:30" otherwise.
pub fn slot_label(start: NaiveDateTime, interval: u32) -> String {
    if interval >= DAY_MINUTES {
        start.format("%-m月%-d日").to_string()
    } else {
        start.format("%-m月%-d日 %H:%M").to_string()
    }
}

/// Downloads a catalogue work and checks its shape and contrast again, since Commons
/// files can be replaced. How it reads on the panel was judged when the catalogue
/// was built.
pub fn download(client: &Client, entry: &Entry) -> Result<(ArtMeta, GrayImage), FetchError> {
    let bytes = client.get(&entry.image, &[], MAX_IMAGE)?;
    let img = image::load_from_memory(&bytes)
        .map_err(|e| FetchError::Unusable(format!("图片无法解码：{e}")))?
        .to_luma8();
    let fit = art::check_quality(&img, entry.meta.focus, W, ART_H).map_err(FetchError::Unusable)?;
    let mut meta = entry.meta.clone();
    meta.focus = fit.focus;
    Ok((meta, shrink(img, KEEP_SIDE)))
}

pub struct Syncer {
    cache: Cache,
    client: Client,
    catalog: Arc<Vec<Entry>>,
    store: Arc<RwLock<Store>>,
    failures: HashMap<String, u32>,
}

impl Syncer {
    pub fn new(cache: Cache, catalog: Arc<Vec<Entry>>, store: Arc<RwLock<Store>>) -> Self {
        Self { cache, client: Client::new(), catalog, store, failures: HashMap::new() }
    }

    /// Makes sure the current slot, the next one and the `behind` slots before it each
    /// have a work, calling `ready` after each change; in fixed mode nothing is
    /// fetched. Stops at the first network failure; the next call carries on.
    pub fn sync(
        &mut self,
        now: NaiveDateTime,
        behind: i64,
        log: &mut dyn FnMut(String),
        ready: &mut dyn FnMut(),
    ) -> Result<(), String> {
        let (settings, own, gallery) = {
            let store = store::read(&self.store);
            (store.settings().clone(), store.rotating_gallery(), store.gallery().to_vec())
        };
        let mut state = self.cache.state();
        let forgotten = self.forget_unknown(&mut state, &gallery);
        if forgotten > 0 {
            log(format!("{forgotten} 个时段的画已不在目录或图库里，换掉"));
            self.cache.save_state(&state).map_err(|e| format!("保存状态失败：{e}"))?;
            ready();
        }
        let current = slot_start(now, settings.interval);
        if settings.mode == Mode::Rotate {
            let mut slots = vec![current, next_slot(current, settings.interval)];
            let mut slot = current;
            for _ in 0..behind {
                slot = previous_slot(slot, settings.interval);
                slots.push(slot);
            }
            for slot in slots {
                if state.slots.get(&slot).is_some_and(|k| self.available(k)) {
                    continue;
                }
                self.fill(slot, &settings, &own, &mut state, log)?;
                ready();
            }
        }
        self.cleanup(current, &mut state).map_err(|e| format!("清理缓存失败：{e}"))
    }

    /// Forgets the current slot and those after it, so they are filled again under
    /// new settings.
    pub fn reset(&mut self, now: NaiveDateTime) -> Result<(), String> {
        let current = slot_start(now, store::read(&self.store).settings().interval);
        let mut state = self.cache.state();
        let before = state.slots.len();
        state.slots.retain(|start, _| *start < current);
        if state.slots.len() != before {
            self.cache.save_state(&state).map_err(|e| format!("保存状态失败：{e}"))?;
        }
        Ok(())
    }

    fn available(&self, key: &str) -> bool {
        if store::is_own(key) {
            store::read(&self.store).files().has_work(key)
        } else {
            self.cache.has_work(key)
        }
    }

    /// Drops slots whose work is neither in the catalogue nor in the gallery, such as
    /// museum picks from older versions, works taken out on review or deleted
    /// pictures, and returns how many.
    fn forget_unknown(&self, state: &mut State, gallery: &[String]) -> usize {
        let catalog: HashSet<&str> = self.catalog.iter().map(|e| e.meta.id.as_str()).collect();
        let before = state.slots.len();
        state
            .slots
            .retain(|_, k| catalog.contains(k.as_str()) || gallery.iter().any(|g| g == k));
        state.rejected.retain(|k| catalog.contains(k.as_str()));
        before - state.slots.len()
    }

    fn fill(
        &mut self,
        slot: NaiveDateTime,
        settings: &Settings,
        own: &[String],
        state: &mut State,
        log: &mut dyn FnMut(String),
    ) -> Result<(), String> {
        let label = slot_label(slot, settings.interval);
        let has_catalog = self.catalog.iter().any(|e| !settings.skip.contains(&e.meta.id));
        let c = if has_catalog { settings.catalog_share } else { 0 };
        let g = if own.is_empty() { 0 } else { settings.gallery_share };
        let n = slot_number(slot, settings.interval);
        let choice = match (c, g) {
            (0, 0) if !own.is_empty() => Pick::Gallery(n),
            (0, 0) if has_catalog => Pick::Catalog(n),
            (0, 0) => return Err("目录和图库里都没有可以轮播的画".into()),
            _ => pick(n, c, g),
        };
        let start = match choice {
            Pick::Gallery(i) => {
                let key = own[i.rem_euclid(own.len() as i64) as usize].clone();
                let title = store::read(&self.store)
                    .files()
                    .load_meta(&key)
                    .map(|m| m.title)
                    .unwrap_or_default();
                log(format!("{label}：我的图库《{title}》"));
                state.slots.insert(slot, key);
                return self.cache.save_state(state).map_err(|e| format!("保存状态失败：{e}"));
            }
            Pick::Catalog(i) => i,
        };
        let used: HashSet<String> = state
            .slots
            .iter()
            .filter(|(s, _)| **s != slot)
            .map(|(_, k)| k.clone())
            .collect();
        let mut tries = 0;
        for i in order(&self.catalog, start) {
            let entry = &self.catalog[i];
            let key = entry.meta.id.clone();
            if state.rejected.contains(&key)
                || used.contains(&key)
                || settings.skip.contains(&key)
                || self.failures.get(&key).is_some_and(|&n| n >= MAX_FAILURES)
            {
                continue;
            }
            if self.cache.has_work(&key) {
                log(format!("{label}：《{}》，{}（已缓存）", entry.meta.title, entry.meta.artist));
                state.slots.insert(slot, key);
                return self.cache.save_state(state).map_err(|e| format!("保存状态失败：{e}"));
            }
            if tries == TRIES_PER_SLOT {
                break;
            }
            tries += 1;
            match download(&self.client, entry) {
                Ok((meta, img)) => {
                    self.cache.save_work(&meta, &img).map_err(|e| format!("保存 {key} 失败：{e}"))?;
                    log(format!("{label}：《{}》，{}", meta.title, meta.artist));
                    state.slots.insert(slot, key);
                    self.cache.save_state(state).map_err(|e| format!("保存状态失败：{e}"))?;
                    return Ok(());
                }
                Err(FetchError::Unusable(why)) => {
                    log(format!("跳过 {key}《{}》：{why}", entry.meta.title));
                    state.rejected.insert(key);
                    self.cache.save_state(state).map_err(|e| format!("保存状态失败：{e}"))?;
                }
                Err(FetchError::Transient(why)) => {
                    *self.failures.entry(key).or_default() += 1;
                    return Err(why);
                }
            }
        }
        Err(format!("{label} 没找到能用的画"))
    }

    /// Keeps the `KEEP` latest slots up to `current` and those after it, and deletes
    /// downloaded works no slot uses any more.
    fn cleanup(&self, current: NaiveDateTime, state: &mut State) -> io::Result<()> {
        let past: Vec<NaiveDateTime> = state.slots.range(..=current).map(|(s, _)| *s).collect();
        let before = state.slots.len();
        for start in past.iter().take(past.len().saturating_sub(KEEP)) {
            state.slots.remove(start);
        }
        if state.slots.len() != before {
            self.cache.save_state(state)?;
        }
        let keep: HashSet<&str> = state.slots.values().map(String::as_str).collect();
        let Ok(entries) = fs::read_dir(self.cache.works()) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            if !keep.contains(stem) {
                fs::remove_file(&path)?;
            }
        }
        // Left over from versions that picked from museum collections at random.
        let _ = fs::remove_file(self.cache.dir.join("pool.json"));
        Ok(())
    }
}

/// Indices into `catalog` in the order to try for the slot at `position` in the
/// catalogue's sequence. The famous works and the rest are each shuffled once, then
/// interleaved evenly so the extras never bunch up; the sequence is walked one step a
/// slot, so nothing repeats until it runs out.
fn order(catalog: &[Entry], position: i64) -> Vec<usize> {
    let shuffled = |famous: bool| {
        let mut ids: Vec<usize> = (0..catalog.len())
            .filter(|&i| (catalog[i].group == FAMOUS) == famous)
            .collect();
        ids.sort_by_key(|&i| (mix(fnv(&catalog[i].meta.id)), catalog[i].meta.id.clone()));
        ids
    };
    let mut seq: Vec<(f64, usize)> = Vec::with_capacity(catalog.len());
    for list in [shuffled(true), shuffled(false)] {
        let n = list.len() as f64;
        seq.extend(list.into_iter().enumerate().map(|(k, i)| ((k as f64 + 0.5) / n, i)));
    }
    seq.sort_by(|a, b| a.0.total_cmp(&b.0));
    if seq.is_empty() {
        return Vec::new();
    }
    let start = position.rem_euclid(seq.len() as i64) as usize;
    (0..seq.len()).map(|k| seq[(start + k) % seq.len()].1).collect()
}

/// FNV-1a, so the order depends only on the IDs.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

/// SplitMix64: a stable hash, so the shuffled order survives restarts and upgrades.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn shrink(img: GrayImage, side: u32) -> GrayImage {
    if img.width().max(img.height()) <= side {
        return img;
    }
    let (w, h) = art::fit_size(img.width(), img.height(), side, side);
    imageops::resize(&img, w, h, imageops::FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<Entry> {
        (0..60)
            .map(|n| Entry {
                meta: ArtMeta { id: format!("Q{n}"), ..ArtMeta::default() },
                image: String::new(),
                fame: 0,
                group: if n < 40 { FAMOUS.into() } else { "版画".into() },
                midtones: 0.0,
            })
            .collect()
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("art-clock-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn slots_do_not_repeat_until_the_list_runs_out() {
        let list = catalog();
        let picks: Vec<usize> = (0..60).map(|n| order(&list, 739_000 + n)[0]).collect();
        assert_eq!(picks.iter().collect::<HashSet<_>>().len(), 60);
        assert_eq!(order(&list, 0).len(), 60);
    }

    #[test]
    fn extras_are_spread_between_famous_works() {
        let list = catalog();
        let seq = order(&list, 739_000);
        let famous: Vec<bool> = seq.iter().map(|&i| list[i].group == FAMOUS).collect();
        for w in famous.windows(2) {
            assert!(w[0] || w[1], "two extras in a row");
        }
    }

    #[test]
    fn slots_follow_the_interval() {
        let t = at("2026-11-06 14:37");
        assert_eq!(slot_start(t, 1440), at("2026-11-06 00:00"));
        assert_eq!(slot_start(t, 15), at("2026-11-06 14:30"));
        assert_eq!(slot_start(t, 180), at("2026-11-06 12:00"));
        assert_eq!(next_slot(at("2026-11-06 23:45"), 15), at("2026-11-07 00:00"));
        assert_eq!(next_slot(at("2026-11-06 00:00"), 1440), at("2026-11-07 00:00"));
        assert_eq!(previous_slot(at("2026-11-07 00:00"), 15), at("2026-11-06 23:45"));
        assert_eq!(previous_slot(at("2026-11-07 00:00"), 1440), at("2026-11-06 00:00"));
        // Daily slots keep the day numbers the rotation used before slots existed.
        let day = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        assert_eq!(slot_number(at("2026-11-06 00:00"), 1440), day.num_days_from_ce() as i64);
        let n = slot_number(at("2026-11-06 23:45"), 15);
        assert_eq!(slot_number(at("2026-11-07 00:00"), 15), n + 1);
    }

    #[test]
    fn shares_are_spread_evenly() {
        let picks: Vec<Pick> = (0..8).map(|n| pick(n, 3, 1)).collect();
        let own: Vec<i64> = picks.iter().filter_map(|p| match p {
            Pick::Gallery(i) => Some(*i),
            _ => None,
        }).collect();
        assert_eq!(own, [0, 1]);
        assert_eq!(picks[3], Pick::Gallery(0));
        assert_eq!(picks[4], Pick::Catalog(3));
        let alternate: Vec<Pick> = (10..14).map(|n| pick(n, 1, 1)).collect();
        assert_eq!(alternate, [Pick::Catalog(5), Pick::Gallery(5), Pick::Catalog(6), Pick::Gallery(6)]);
        assert_eq!(pick(7, 2, 0), Pick::Catalog(7));
        assert_eq!(pick(7, 0, 2), Pick::Gallery(7));
    }

    #[test]
    fn old_daily_state_becomes_slots() {
        let dir = temp("migrate");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("state.json"), r#"{"days":{"2026-11-06":"Q1"},"rejected":["Q2"]}"#).unwrap();
        let state = Cache::new(dir.clone()).state();
        assert_eq!(state.slots.get(&at("2026-11-06 00:00")).map(String::as_str), Some("Q1"));
        assert!(state.rejected.contains("Q2"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lineup_puts_the_fixed_picture_first_and_falls_back_to_earlier_slots() {
        let mut state = State::default();
        state.slots.insert(at("2026-11-05 00:00"), "Q5".into());
        state.slots.insert(at("2026-11-04 00:00"), "Q4".into());
        state.slots.insert(at("2026-11-07 00:00"), "Q7".into());
        let mut settings = Settings::default();
        let now = at("2026-11-06 10:00");
        assert_eq!(lineup(&state, &settings, now), ["Q5", "Q4"], "today's is not there yet");
        settings.mode = Mode::Fixed;
        settings.fixed = Some("my-1".into());
        assert_eq!(lineup(&state, &settings, now), ["my-1", "Q5", "Q4"]);
    }

    #[test]
    fn slots_outside_the_catalogue_and_gallery_are_forgotten() {
        let dir = temp("forget");
        let store = Arc::new(RwLock::new(Store::open(dir.join("data"))));
        let syncer = Syncer::new(Cache::new(dir.join("cache")), Arc::new(catalog()), store);
        let mut state = State::default();
        let day = at("2026-11-06 00:00");
        state.slots.insert(day, "Q1".into());
        state.slots.insert(day - TimeDelta::days(1), "met-436851".into());
        state.slots.insert(day - TimeDelta::days(2), "my-1".into());
        state.slots.insert(day - TimeDelta::days(3), "my-2".into());
        state.rejected.insert("cma-143142".into());
        assert_eq!(syncer.forget_unknown(&mut state, &["my-1".into()]), 2);
        assert_eq!(state.slots.len(), 2);
        assert!(state.rejected.is_empty());
    }

    #[test]
    fn own_pictures_take_their_turns_without_downloads() {
        let dir = temp("own");
        let store = Arc::new(RwLock::new(Store::open(dir.join("data"))));
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        let key = store.write().unwrap().add_picture(ArtMeta { title: "猫".into(), ..ArtMeta::default() }, &img).unwrap();
        {
            let mut s = store.write().unwrap();
            let mut settings = s.settings().clone();
            settings.catalog_share = 0;
            settings.interval = 60;
            s.save_settings(settings).unwrap();
        }
        let mut syncer = Syncer::new(Cache::new(dir.join("cache")), Arc::new(catalog()), store);
        let mut logs = Vec::new();
        syncer.sync(at("2026-11-06 14:20"), 2, &mut |m| logs.push(m), &mut || {}).unwrap();
        let state = syncer.cache.state();
        assert_eq!(state.slots.len(), 4, "{logs:?}");
        assert!(state.slots.values().all(|k| *k == key));
        assert!(state.slots.contains_key(&at("2026-11-06 15:00")), "the next slot is ready ahead of time");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reset_forgets_the_current_slot_and_later_ones() {
        let dir = temp("reset");
        let store = Arc::new(RwLock::new(Store::open(dir.join("data"))));
        let cache = Cache::new(dir.join("cache"));
        let mut state = State::default();
        for (t, k) in [("2026-11-05 00:00", "Q1"), ("2026-11-06 00:00", "Q2"), ("2026-11-07 00:00", "Q3")] {
            state.slots.insert(at(t), k.into());
        }
        cache.save_state(&state).unwrap();
        let mut syncer = Syncer::new(cache.clone(), Arc::new(catalog()), store);
        syncer.reset(at("2026-11-06 09:00")).unwrap();
        assert_eq!(cache.state().slots.values().collect::<Vec<_>>(), ["Q1"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_keeps_only_recent_slots() {
        let dir = temp("cleanup");
        let cache = Cache::new(dir.join("cache"));
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        let today = at("2026-11-20 00:00");
        let mut state = State::default();
        for n in 0..20 {
            let key = format!("Q{n}");
            cache.save_work(&ArtMeta { id: key.clone(), ..ArtMeta::default() }, &img).unwrap();
            state.slots.insert(today - TimeDelta::days(n), key);
        }
        state.slots.insert(today + TimeDelta::days(1), "Q0".into());
        let store = Arc::new(RwLock::new(Store::open(dir.join("data"))));
        let syncer = Syncer::new(cache.clone(), Arc::new(catalog()), store);
        syncer.cleanup(today, &mut state).unwrap();
        assert!(cache.has_work("Q0") && cache.has_work("Q13"));
        assert!(!cache.has_work("Q14"));
        assert_eq!(cache.state().slots.len(), KEEP + 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_keeps_the_newest_and_the_named() {
        let dir = temp("prune");
        let cache = Cache::new(dir.clone());
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        for n in 0..4 {
            cache.save_work(&ArtMeta { id: format!("Q{n}"), ..ArtMeta::default() }, &img).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        cache.prune(2, &["Q0"]).unwrap();
        assert!(cache.has_work("Q0") && cache.has_work("Q2") && cache.has_work("Q3"));
        assert!(!cache.has_work("Q1"));
        let _ = fs::remove_dir_all(&dir);
    }
}
