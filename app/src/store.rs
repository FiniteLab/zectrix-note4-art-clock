//! The owner's own data, kept apart from the download cache: how pictures take
//! turns, the pictures they added, their edits to the texts, the work they keep on
//! screen and the admin password.
//!
//! Data directory (`$XDG_DATA_HOME/art-clock`, else `~/.local/share/art-clock`):
//!   settings.json      how pictures take turns
//!   gallery.json       own pictures in the order they were added
//!   password.json      salted hash of the admin password, when one is set
//!   works/<key>.json   own pictures ("my-1", "my-2", …), and a copy of the fixed
//!   works/<key>.jpg    work when it comes from the catalogue
//!   edits/<id>.json    edited texts of catalogue and built-in works
use crate::{
    art::ArtMeta,
    library::{read_json, write_atomic, Cache},
};
use image::GrayImage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs, io,
    path::{Path, PathBuf},
    sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

/// Shortest time a picture stays, in minutes; the panel takes a few seconds to change.
pub const MIN_INTERVAL: u32 = 5;
pub const MAX_INTERVAL: u32 = 24 * 60;
pub const MAX_SHARE: u32 = 20;
const OWN_PREFIX: &str = "my-";
const HASH_ROUNDS: u32 = 20_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A new picture every `interval` minutes.
    #[default]
    Rotate,
    /// The `fixed` picture stays.
    Fixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mode: Mode,
    /// Minutes each picture stays; each day's slots start at midnight.
    pub interval: u32,
    /// Of every `catalog_share + gallery_share` slots, this many go to catalogue
    /// works and the rest to own pictures. Zero leaves that source out.
    pub catalog_share: u32,
    pub gallery_share: u32,
    /// Catalogue works and own pictures left out of the rotation.
    pub skip: BTreeSet<String>,
    /// The work kept on screen in fixed mode.
    pub fixed: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Rotate,
            interval: MAX_INTERVAL,
            catalog_share: 1,
            gallery_share: 1,
            skip: BTreeSet::new(),
            fixed: None,
        }
    }
}

impl Settings {
    /// The work to show whatever the time, in fixed mode.
    pub fn fixed_key(&self) -> Option<&str> {
        match self.mode {
            Mode::Fixed => self.fixed.as_deref(),
            Mode::Rotate => None,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Gallery {
    keys: Vec<String>,
    /// Number of the last picture added; keys are never reused, so cached
    /// thumbnails and slots cannot point at a different picture.
    last: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Password {
    salt: String,
    hash: String,
}

pub struct Store {
    dir: PathBuf,
    files: Cache,
    settings: Settings,
    gallery: Gallery,
    password: Option<Password>,
}

/// Own pictures have keys like "my-3"; catalogue works use Wikidata IDs.
pub fn is_own(key: &str) -> bool {
    key.strip_prefix(OWN_PREFIX).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The lock only guards plain data, so a panic elsewhere leaves it usable.
pub fn read(store: &RwLock<Store>) -> RwLockReadGuard<'_, Store> {
    store.read().unwrap_or_else(PoisonError::into_inner)
}

pub fn write(store: &RwLock<Store>) -> RwLockWriteGuard<'_, Store> {
    store.write().unwrap_or_else(PoisonError::into_inner)
}

impl Store {
    /// `$XDG_DATA_HOME/art-clock`, else `~/.local/share/art-clock`.
    pub fn default_dir() -> PathBuf {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
            .unwrap_or_else(std::env::temp_dir)
            .join("art-clock")
    }

    /// Missing or unreadable files count as defaults, so a fresh install just works.
    pub fn open(dir: PathBuf) -> Self {
        let mut settings: Settings = read_json(&dir.join("settings.json")).unwrap_or_default();
        settings.interval = settings.interval.clamp(MIN_INTERVAL, MAX_INTERVAL);
        Self {
            files: Cache::new(dir.clone()),
            gallery: read_json(&dir.join("gallery.json")).unwrap_or_default(),
            password: read_json(&dir.join("password.json")),
            settings,
            dir,
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Saves new settings and drops kept copies of catalogue works no longer fixed.
    pub fn save_settings(&mut self, settings: Settings) -> io::Result<()> {
        write_atomic(&self.dir.join("settings.json"), &serde_json::to_vec_pretty(&settings)?)?;
        self.settings = settings;
        let fixed = self.settings.fixed.clone();
        let own: Vec<&str> = self.gallery.keys.iter().map(String::as_str).chain(fixed.as_deref()).collect();
        self.files.prune(0, &own)
    }

    /// Own pictures, oldest first.
    pub fn gallery(&self) -> &[String] {
        &self.gallery.keys
    }

    /// Own pictures that take turns with the catalogue.
    pub fn rotating_gallery(&self) -> Vec<String> {
        self.gallery.keys.iter().filter(|k| !self.settings.skip.contains(*k)).cloned().collect()
    }

    /// Own pictures and the kept copy of the fixed work.
    pub fn files(&self) -> &Cache {
        &self.files
    }

    pub fn add_picture(&mut self, mut meta: ArtMeta, img: &GrayImage) -> io::Result<String> {
        let key = format!("{OWN_PREFIX}{}", self.gallery.last + 1);
        meta.id = key.clone();
        self.files.save_work(&meta, img)?;
        self.gallery.last += 1;
        self.gallery.keys.push(key.clone());
        self.save_gallery()?;
        Ok(key)
    }

    /// Deletes an own picture; when it was fixed on screen, rotation takes over.
    pub fn remove_picture(&mut self, key: &str) -> io::Result<()> {
        self.gallery.keys.retain(|k| k != key);
        self.save_gallery()?;
        self.files.remove_work(key)?;
        let mut settings = self.settings.clone();
        settings.skip.remove(key);
        if settings.fixed.as_deref() == Some(key) {
            settings.fixed = None;
            settings.mode = Mode::Rotate;
        }
        if settings != self.settings {
            self.save_settings(settings)?;
        }
        Ok(())
    }

    fn save_gallery(&self) -> io::Result<()> {
        write_atomic(&self.dir.join("gallery.json"), &serde_json::to_vec_pretty(&self.gallery)?)
    }

    fn edit_path(&self, id: &str) -> PathBuf {
        self.dir.join("edits").join(format!("{id}.json"))
    }

    /// The owner's version of a catalogue or built-in work's texts.
    pub fn edit(&self, id: &str) -> Option<ArtMeta> {
        if is_own(id) {
            return None;
        }
        read_json(&self.edit_path(id))
    }

    /// IDs of the works whose texts the owner changed.
    pub fn edited(&self) -> BTreeSet<String> {
        let Ok(entries) = fs::read_dir(self.dir.join("edits")) else {
            return BTreeSet::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let stem = path.file_stem()?.to_str()?;
                (path.extension()? == "json").then(|| stem.to_string())
            })
            .collect()
    }

    /// Own pictures keep their texts with them; other works get an edit file.
    pub fn save_edit(&self, meta: &ArtMeta) -> io::Result<()> {
        if is_own(&meta.id) {
            self.files.save_meta(meta)
        } else {
            write_atomic(&self.edit_path(&meta.id), &serde_json::to_vec_pretty(meta)?)
        }
    }

    pub fn remove_edit(&self, id: &str) -> io::Result<()> {
        match fs::remove_file(self.edit_path(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// The texts to show for a work: the owner's edit when there is one.
    pub fn apply(&self, meta: ArtMeta) -> ArtMeta {
        match self.edit(&meta.id) {
            Some(edit) => ArtMeta { id: meta.id, file: meta.file, ..edit },
            None => meta,
        }
    }

    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    pub fn check_password(&self, password: &str) -> bool {
        self.password.as_ref().is_none_or(|p| same(&hash(&p.salt, password), &p.hash))
    }

    /// Sets the admin password, or removes it with `None` or an empty one.
    pub fn set_password(&mut self, password: Option<&str>) -> io::Result<()> {
        let path = self.dir.join("password.json");
        match password.filter(|p| !p.is_empty()) {
            Some(p) => {
                let salt = random_hex(16);
                let saved = Password { hash: hash(&salt, p), salt };
                write_atomic(&path, &serde_json::to_vec_pretty(&saved)?)?;
                self.password = Some(saved);
            }
            None => {
                match fs::remove_file(&path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
                self.password = None;
            }
        }
        Ok(())
    }

    /// The login cookie's value. Derived from the password hash, so logins survive
    /// restarts and end when the password changes; `None` without a password.
    pub fn session_token(&self) -> Option<String> {
        let p = self.password.as_ref()?;
        Some(hex(&Sha256::digest(format!("session\n{}\n{}", p.salt, p.hash))))
    }
}

/// Compares in time that does not depend on where the strings differ.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hash(salt: &str, password: &str) -> String {
    let mut h = Sha256::digest(format!("{salt}\n{password}"));
    for _ in 0..HASH_ROUNDS {
        let mut next = Sha256::new();
        next.update(h);
        next.update(password.as_bytes());
        h = next.finalize();
    }
    hex(&h)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn random_hex(n: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    if fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).is_err() {
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        buf = Sha256::digest(seed)[..n.min(32)].to_vec();
    }
    hex(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("art-clock-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn settings_and_pictures_survive_a_restart() {
        let dir = temp("restart");
        let mut store = Store::open(dir.clone());
        assert_eq!(*store.settings(), Settings::default());
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        let a = store.add_picture(ArtMeta { title: "猫".into(), ..ArtMeta::default() }, &img).unwrap();
        let b = store.add_picture(ArtMeta { title: "狗".into(), ..ArtMeta::default() }, &img).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("my-1", "my-2"));
        let mut settings = store.settings().clone();
        settings.interval = 15;
        settings.skip.insert(a.clone());
        store.save_settings(settings).unwrap();
        let store = Store::open(dir.clone());
        assert_eq!(store.settings().interval, 15);
        assert_eq!(store.gallery(), [a.clone(), b.clone()]);
        assert_eq!(store.rotating_gallery(), std::slice::from_ref(&b));
        assert_eq!(store.files().load_meta(&a).unwrap().title, "猫");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removed_pictures_free_their_key_but_never_reuse_it() {
        let dir = temp("remove");
        let mut store = Store::open(dir.clone());
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        let a = store.add_picture(ArtMeta::default(), &img).unwrap();
        let mut settings = store.settings().clone();
        settings.mode = Mode::Fixed;
        settings.fixed = Some(a.clone());
        store.save_settings(settings).unwrap();
        store.remove_picture(&a).unwrap();
        assert!(!store.files().has_work(&a));
        assert_eq!(store.settings().mode, Mode::Rotate, "rotation takes over");
        assert_eq!(store.settings().fixed, None);
        assert_eq!(store.add_picture(ArtMeta::default(), &img).unwrap(), "my-2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_fixed_catalogue_copy_is_kept() {
        let dir = temp("keep");
        let mut store = Store::open(dir.clone());
        let img = GrayImage::from_pixel(8, 8, image::Luma([128]));
        let own = store.add_picture(ArtMeta::default(), &img).unwrap();
        for id in ["Q1", "Q2"] {
            store.files().save_work(&ArtMeta { id: id.into(), ..ArtMeta::default() }, &img).unwrap();
        }
        let mut settings = store.settings().clone();
        settings.fixed = Some("Q2".into());
        store.save_settings(settings).unwrap();
        assert!(store.files().has_work(&own) && store.files().has_work("Q2"));
        assert!(!store.files().has_work("Q1"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_replace_the_texts_but_keep_the_id() {
        let dir = temp("edit");
        let store = Store::open(dir.clone());
        let original = ArtMeta { id: "Q1".into(), title: "星夜".into(), intro: Some("原文".into()), ..ArtMeta::default() };
        assert_eq!(store.apply(original.clone()).intro.as_deref(), Some("原文"));
        let edit = ArtMeta { intro: Some("我的讲解".into()), ..original.clone() };
        store.save_edit(&edit).unwrap();
        assert!(store.edited().contains("Q1"));
        let shown = store.apply(original.clone());
        assert_eq!((shown.id.as_str(), shown.intro.as_deref()), ("Q1", Some("我的讲解")));
        store.remove_edit("Q1").unwrap();
        assert_eq!(store.apply(original).intro.as_deref(), Some("原文"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn passwords_are_optional_and_checked() {
        let dir = temp("password");
        let mut store = Store::open(dir.clone());
        assert!(store.check_password("anything") && store.session_token().is_none());
        store.set_password(Some("芝麻开门")).unwrap();
        let store2 = Store::open(dir.clone());
        assert!(store2.check_password("芝麻开门"));
        assert!(!store2.check_password("芝麻关门"));
        assert_eq!(store2.session_token(), store.session_token());
        store.set_password(Some("")).unwrap();
        assert!(!store.has_password() && !Store::open(dir.clone()).has_password());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn own_keys_are_recognised() {
        assert!(is_own("my-1") && is_own("my-120"));
        assert!(!is_own("my-") && !is_own("Q12") && !is_own("rainy") && !is_own("my-a"));
    }
}
