//! The admin page: a small web server inside the clock for choosing how pictures
//! take turns, adding the owner's own pictures and editing the texts.
//!
//! Every work is addressed by its key: a Wikidata ID for catalogue works, "my-N"
//! for own pictures, the file stem for the built-ins. With a password set, every
//! request but the page itself and the login needs the login cookie.
use crate::{
    art::{self, ArtMeta},
    catalog::Entry,
    layout::{self, Fonts, ART_H},
    library::{self, write_atomic, Cache},
    net::Client,
    render::W,
    store::{self, Mode, Store, MAX_INTERVAL, MAX_SHARE, MIN_INTERVAL},
};
use chrono::Local;
use image::{
    codecs::{jpeg::JpegEncoder, png::PngEncoder},
    DynamicImage, ExtendedColorType, GrayImage, ImageDecoder, ImageEncoder, ImageReader, Luma,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{Cursor, Read},
    sync::{Arc, Mutex, PoisonError, RwLock},
    thread,
    time::Duration,
};
use tiny_http::{Header, Method, Request, Response, Server};

const PAGE: &str = include_str!("admin.html");
pub const DEFAULT_PORT: u16 = 8686;
const MAX_UPLOAD: usize = 40 << 20;
const MAX_JSON: usize = 1 << 20;
const THUMB_SIDE: u32 = 330;
const COOKIE: &str = "huashi";
/// Catalogue works downloaded for a preview, kept so the next preview is quick.
const RECENT_DOWNLOADS: usize = 6;
/// Until a device reports its own: sixteen even steps of linear light.
const UNIFORM: [u8; 16] = [0, 17, 34, 51, 68, 85, 102, 119, 136, 153, 170, 187, 204, 221, 238, 255];

/// What the clock is doing, for the page.
#[derive(Debug, Default, Clone)]
pub struct Live {
    /// Key of the work on screen.
    pub showing: Option<String>,
    pub connected: bool,
    /// The panel's sixteen greys in linear light, as the device reported them.
    pub palette: Option<[u8; 16]>,
}

/// What a change made on the page asks of the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// How pictures take turns changed: fill the current slot again, then redraw.
    Rotation,
    /// Texts changed: redraw.
    Texts,
}

pub struct Admin {
    pub store: Arc<RwLock<Store>>,
    pub cache: Cache,
    pub catalog: Arc<Vec<Entry>>,
    pub builtins: Arc<Vec<(ArtMeta, GrayImage)>>,
    pub fonts: Arc<Fonts>,
    pub live: Arc<Mutex<Live>>,
    pub notify: Box<dyn Fn(Change) + Send + Sync>,
    /// Nothing is downloaded.
    pub offline: bool,
}

struct Web {
    admin: Admin,
    client: Client,
    downloads: Mutex<VecDeque<(ArtMeta, GrayImage)>>,
}

type Reply = Response<Cursor<Vec<u8>>>;

struct Fail {
    status: u16,
    message: String,
}

fn fail(status: u16, message: impl Into<String>) -> Fail {
    Fail { status, message: message.into() }
}

fn saving(e: std::io::Error) -> Fail {
    fail(500, format!("保存失败：{e}"))
}

/// Starts the server on its own thread, one more thread per request.
pub fn serve(admin: Admin, addr: &str) -> Result<(), String> {
    let server = Server::http(addr).map_err(|e| format!("管理网页无法监听 {addr}：{e}"))?;
    let web = Arc::new(Web { admin, client: Client::new(), downloads: Mutex::new(VecDeque::new()) });
    thread::Builder::new()
        .name("web".into())
        .spawn(move || {
            for req in server.incoming_requests() {
                let web = web.clone();
                let _ = thread::Builder::new().name("web-request".into()).spawn(move || web.handle(req));
            }
        })
        .map_err(|e| format!("管理网页启动失败：{e}"))?;
    Ok(())
}

#[derive(Deserialize)]
struct Login {
    password: String,
}

#[derive(Deserialize)]
struct NewSettings {
    mode: Mode,
    interval: u32,
    catalog_share: u32,
    gallery_share: u32,
    fixed: Option<String>,
}

#[derive(Deserialize)]
struct Skip {
    key: String,
    skip: bool,
}

#[derive(Deserialize)]
struct KeyOnly {
    key: String,
}

#[derive(Deserialize)]
struct NewPassword {
    #[serde(default)]
    current: String,
    #[serde(default)]
    new: String,
}

/// The texts and crop the page edits; everything else about a work stays.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Edit {
    pub title: String,
    pub artist: String,
    pub year: String,
    pub artist_about: String,
    pub genre: String,
    pub movement: String,
    pub medium: String,
    pub dimensions: String,
    pub museum: String,
    pub intro_heading: String,
    pub intro: String,
    pub intro_source: String,
    /// Where the crop sits, 0 = top, 1 = bottom; pictures as wide as the band have none.
    pub focus: Option<f32>,
    pub whole: bool,
}

impl Edit {
    pub fn apply(&self, meta: &mut ArtMeta) {
        let line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
        let opt = |s: &str| Some(line(s)).filter(|s| !s.is_empty());
        meta.title = opt(&self.title).unwrap_or_else(|| "无题".into());
        meta.artist = line(&self.artist);
        meta.year = line(&self.year);
        meta.artist_about = opt(&self.artist_about);
        meta.genre = opt(&self.genre);
        meta.movement = opt(&self.movement);
        meta.medium = opt(&self.medium);
        meta.dimensions = opt(&self.dimensions);
        meta.museum = opt(&self.museum);
        meta.intro_heading = opt(&self.intro_heading);
        let intro: Vec<&str> = self.intro.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        meta.intro = Some(intro.join("\n")).filter(|s| !s.is_empty());
        meta.intro_source = opt(&self.intro_source);
        meta.focus = self.focus.filter(|f| f.is_finite()).map(|y| [0.5, y.clamp(0.0, 1.0)]);
        meta.whole = self.whole;
    }
}

#[derive(Deserialize)]
struct SaveWork {
    key: String,
    edit: Edit,
}

#[derive(Deserialize)]
struct RenderWork {
    key: String,
    #[serde(default)]
    edit: Option<Edit>,
    /// 0 for the clock, 1 and up for the guide pages.
    #[serde(default)]
    page: usize,
}

impl Web {
    fn handle(&self, mut req: Request) {
        let reply = self.route(&mut req).unwrap_or_else(|f| {
            json_reply(f.status, &json!({ "error": f.message }))
        });
        let _ = req.respond(reply);
    }

    fn route(&self, req: &mut Request) -> Result<Reply, Fail> {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let method = req.method().clone();
        if method == Method::Post && !same_origin(req) {
            return Err(fail(403, "请求来自别的网站"));
        }
        match (&method, path) {
            (Method::Get, "/") => return Ok(reply(200, "text/html; charset=utf-8", PAGE.as_bytes().to_vec())),
            (Method::Post, "/api/login") => return self.login(req),
            _ => {}
        }
        if !self.authed(req) {
            return Err(fail(401, "需要登录"));
        }
        let key = || {
            query_param(query, "key").filter(|k| valid_key(k)).ok_or_else(|| fail(400, "缺少画作编号"))
        };
        match (&method, path) {
            (Method::Get, "/api/state") => self.state(),
            (Method::Get, "/api/catalog") => self.catalog(),
            (Method::Get, "/api/work") => self.work(&key()?),
            (Method::Get, "/thumb") => self.thumb(&key()?),
            (Method::Post, "/api/logout") => Ok(json_reply(200, &json!({ "ok": true }))
                .with_header(header("Set-Cookie", &format!("{COOKIE}=; Path=/; Max-Age=0")))),
            (Method::Post, "/api/settings") => self.save_settings(json_body(req)?),
            (Method::Post, "/api/skip") => self.skip(json_body(req)?),
            (Method::Post, "/api/password") => self.password(json_body(req)?),
            (Method::Post, "/api/work") => self.save_work(json_body(req)?),
            (Method::Post, "/api/revert") => self.revert(json_body(req)?),
            (Method::Post, "/api/upload") => {
                let title = query_param(query, "title").unwrap_or_default();
                self.upload(req, &title)
            }
            (Method::Post, "/api/delete") => self.delete(json_body(req)?),
            (Method::Post, "/api/render") => self.render(json_body(req)?),
            _ => Err(fail(404, "没有这个地址")),
        }
    }

    fn authed(&self, req: &Request) -> bool {
        let Some(token) = store::read(&self.admin.store).session_token() else {
            return true;
        };
        cookie(req, COOKIE).is_some_and(|c| store::same(&c, &token))
    }

    fn login(&self, req: &mut Request) -> Result<Reply, Fail> {
        let login: Login = json_body(req)?;
        let token = {
            let store = store::read(&self.admin.store);
            if !store.check_password(&login.password) {
                None
            } else {
                Some(store.session_token())
            }
        };
        match token {
            None => {
                // Slows down guessing.
                thread::sleep(Duration::from_secs(1));
                Err(fail(403, "密码不对"))
            }
            Some(token) => {
                let mut r = json_reply(200, &json!({ "ok": true }));
                if let Some(t) = token {
                    r = r.with_header(session_cookie(&t));
                }
                Ok(r)
            }
        }
    }

    fn builtin(&self, key: &str) -> Option<&(ArtMeta, GrayImage)> {
        self.admin.builtins.iter().find(|(m, _)| m.id == key)
    }

    fn entry(&self, key: &str) -> Option<&Entry> {
        self.admin.catalog.iter().find(|e| e.meta.id == key)
    }

    /// A work's texts as the clock shows them, without loading its picture.
    fn describe(&self, store: &Store, key: &str) -> Option<ArtMeta> {
        let meta = if let Some((m, _)) = self.builtin(key) {
            m.clone()
        } else if let Some(m) = store.files().load_meta(key) {
            m
        } else if let Some(m) = self.admin.cache.load_meta(key) {
            m
        } else {
            self.entry(key)?.meta.clone()
        };
        Some(store.apply(meta))
    }

    fn summary(&self, store: &Store, key: &str) -> Value {
        match self.describe(store, key) {
            Some(m) => json!({ "key": key, "title": m.title, "artist": m.artist, "year": m.year }),
            None => json!({ "key": key, "title": "（已删除）", "artist": "", "year": "" }),
        }
    }

    /// A work's picture with the texts as they came, downloading catalogue works
    /// that are not on disk.
    fn raw(&self, key: &str) -> Result<(ArtMeta, GrayImage), Fail> {
        if let Some(work) = self.on_disk(key) {
            return Ok(work);
        }
        if let Some((m, img)) = lock(&self.downloads).iter().find(|(m, _)| m.id == key) {
            return Ok((m.clone(), img.clone()));
        }
        let entry = self.entry(key).ok_or_else(|| fail(404, "没有这幅画"))?;
        if self.admin.offline {
            return Err(fail(503, "离线模式，不能下载这幅画"));
        }
        let (meta, img) = library::download(&self.client, entry)
            .map_err(|e| fail(502, format!("下载《{}》失败：{e}", entry.meta.title)))?;
        let mut downloads = lock(&self.downloads);
        downloads.push_front((meta.clone(), img.clone()));
        downloads.truncate(RECENT_DOWNLOADS);
        Ok((meta, img))
    }

    /// A work's picture and the texts the clock shows with it.
    fn source(&self, key: &str) -> Result<(ArtMeta, GrayImage), Fail> {
        let (meta, img) = self.raw(key)?;
        Ok((store::read(&self.admin.store).apply(meta), img))
    }

    fn state(&self) -> Result<Reply, Fail> {
        let live = lock(&self.admin.live).clone();
        let store = store::read(&self.admin.store);
        let settings = store.settings();
        let now = Local::now().naive_local();
        let current = library::slot_start(now, settings.interval);
        let slots = self.admin.cache.state().slots;
        let recent: Vec<Value> = slots
            .iter()
            .rev()
            .map(|(start, key)| {
                let mut v = self.summary(&store, key);
                v["slot"] = json!(library::slot_label(*start, settings.interval));
                v["when"] = json!(match start.cmp(&current) {
                    std::cmp::Ordering::Greater => "next",
                    std::cmp::Ordering::Equal => "now",
                    std::cmp::Ordering::Less => "past",
                });
                v
            })
            .collect();
        let gallery: Vec<Value> = store
            .gallery()
            .iter()
            .map(|key| {
                let mut v = self.summary(&store, key);
                v["skip"] = json!(settings.skip.contains(key));
                v
            })
            .collect();
        let builtins: Vec<Value> = self.admin.builtins.iter().map(|(m, _)| self.summary(&store, &m.id)).collect();
        let skipped = self.admin.catalog.iter().filter(|e| settings.skip.contains(&e.meta.id)).count();
        Ok(json_reply(
            200,
            &json!({
                "settings": {
                    "mode": settings.mode,
                    "interval": settings.interval,
                    "catalog_share": settings.catalog_share,
                    "gallery_share": settings.gallery_share,
                    "fixed": settings.fixed,
                },
                "limits": { "min_interval": MIN_INTERVAL, "max_interval": MAX_INTERVAL, "max_share": MAX_SHARE },
                "fixed": settings.fixed.as_deref().map(|k| self.summary(&store, k)),
                "showing": live.showing.as_deref().map(|k| self.summary(&store, k)),
                "connected": live.connected,
                "offline": self.admin.offline,
                "next_change": library::slot_label(library::next_slot(current, settings.interval), 1),
                "has_password": store.has_password(),
                "recent": recent,
                "gallery": gallery,
                "builtins": builtins,
                "catalog_size": self.admin.catalog.len(),
                "catalog_skipped": skipped,
            }),
        ))
    }

    fn catalog(&self) -> Result<Reply, Fail> {
        let store = store::read(&self.admin.store);
        let edited = store.edited();
        let skip = &store.settings().skip;
        let list: Vec<Value> = self
            .admin
            .catalog
            .iter()
            .map(|e| {
                let id = &e.meta.id;
                let m = if edited.contains(id) { store.apply(e.meta.clone()) } else { e.meta.clone() };
                json!({
                    "key": id,
                    "title": m.title,
                    "artist": m.artist,
                    "year": m.year,
                    "group": e.group,
                    "fame": e.fame,
                    "museum": m.museum,
                    "original": [m.title_original, m.artist_original],
                    "skip": skip.contains(id),
                    "edited": edited.contains(id),
                })
            })
            .collect();
        Ok(json_reply(200, &Value::Array(list)))
    }

    fn work(&self, key: &str) -> Result<Reply, Fail> {
        let (meta, img) = self.source(key)?;
        let (w, h) = img.dimensions();
        let (look, focus) = art::look(&img, meta.focus, meta.whole, W, ART_H);
        let crop = art::look(&img, meta.focus, false, W, ART_H).1;
        let store = store::read(&self.admin.store);
        let settings = store.settings();
        Ok(json_reply(
            200,
            &json!({
                "key": key,
                "own": store::is_own(key),
                "builtin": self.builtin(key).is_some(),
                "edited": store.edit(key).is_some(),
                "meta": meta,
                "width": w,
                "height": h,
                "wide": art::shows_whole(w, h, W, ART_H),
                "focus": focus.or(crop).map(|f| f[1]),
                "midtones": look.midtones,
                "muddy": look.midtones > art::MAX_MIDTONES,
                "flat": look.spread < 40.0,
                "skip": settings.skip.contains(key),
                "fixed": settings.fixed_key() == Some(key),
            }),
        ))
    }

    fn save_settings(&self, new: NewSettings) -> Result<Reply, Fail> {
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&new.interval) {
            return Err(fail(400, format!("换画间隔应在 {MIN_INTERVAL} 分钟到 24 小时之间")));
        }
        if new.catalog_share > MAX_SHARE || new.gallery_share > MAX_SHARE {
            return Err(fail(400, format!("比例的每一边最多 {MAX_SHARE}")));
        }
        if let Some(key) = &new.fixed {
            if !valid_key(key) {
                return Err(fail(400, "画作编号不对"));
            }
        }
        if new.mode == Mode::Fixed && new.fixed.is_none() {
            return Err(fail(400, "先选一幅要固定显示的画"));
        }
        if let Some(key) = new.fixed.as_deref().filter(|k| self.builtin(k).is_none()) {
            let kept = {
                let store = store::read(&self.admin.store);
                store.files().has_work(key)
            };
            if !kept {
                if store::is_own(key) {
                    return Err(fail(404, "图库里没有这张图"));
                }
                // The cache lets go of works as the rotation moves on.
                let (meta, img) = self.raw(key)?;
                store::read(&self.admin.store).files().save_work(&meta, &img).map_err(saving)?;
            }
        }
        {
            let mut store = store::write(&self.admin.store);
            let mut settings = store.settings().clone();
            settings.mode = new.mode;
            settings.interval = new.interval;
            settings.catalog_share = new.catalog_share;
            settings.gallery_share = new.gallery_share;
            settings.fixed = new.fixed;
            store.save_settings(settings).map_err(saving)?;
        }
        (self.admin.notify)(Change::Rotation);
        self.state()
    }

    fn skip(&self, req: Skip) -> Result<Reply, Fail> {
        {
            let mut store = store::write(&self.admin.store);
            let mut settings = store.settings().clone();
            if req.skip {
                settings.skip.insert(req.key);
            } else {
                settings.skip.remove(&req.key);
            }
            store.save_settings(settings).map_err(saving)?;
        }
        (self.admin.notify)(Change::Rotation);
        Ok(json_reply(200, &json!({ "ok": true })))
    }

    fn password(&self, req: NewPassword) -> Result<Reply, Fail> {
        let token = {
            let mut store = store::write(&self.admin.store);
            if !store.check_password(&req.current) {
                drop(store);
                thread::sleep(Duration::from_secs(1));
                return Err(fail(403, "当前密码不对"));
            }
            store.set_password(Some(req.new.as_str())).map_err(saving)?;
            store.session_token()
        };
        let r = json_reply(200, &json!({ "ok": true, "has_password": token.is_some() }));
        Ok(match token {
            Some(t) => r.with_header(session_cookie(&t)),
            None => r,
        })
    }

    fn save_work(&self, req: SaveWork) -> Result<Reply, Fail> {
        if !valid_key(&req.key) {
            return Err(fail(400, "画作编号不对"));
        }
        let (raw, _) = self.raw(&req.key)?;
        let store = store::read(&self.admin.store);
        let mut meta = store.apply(raw.clone());
        req.edit.apply(&mut meta);
        let unchanged = !store::is_own(&req.key)
            && serde_json::to_value(&meta).ok() == serde_json::to_value(&raw).ok();
        if unchanged {
            store.remove_edit(&req.key).map_err(saving)?;
        } else {
            store.save_edit(&meta).map_err(saving)?;
        }
        drop(store);
        (self.admin.notify)(Change::Texts);
        self.work(&req.key)
    }

    fn revert(&self, req: KeyOnly) -> Result<Reply, Fail> {
        if !valid_key(&req.key) {
            return Err(fail(400, "画作编号不对"));
        }
        store::read(&self.admin.store).remove_edit(&req.key).map_err(saving)?;
        (self.admin.notify)(Change::Texts);
        self.work(&req.key)
    }

    fn upload(&self, req: &mut Request, title: &str) -> Result<Reply, Fail> {
        let bytes = read_body(req, MAX_UPLOAD)?;
        let img = decode_upload(&bytes)?;
        let (w, h) = img.dimensions();
        if w.min(h) < 100 {
            return Err(fail(400, format!("图片太小（{w}×{h}），至少要 100 像素")));
        }
        let img = library::shrink(img, library::KEEP_SIDE);
        // Cropping a tall picture to the band keeps only a sliver of it.
        let whole = (w as f32 / h as f32) < art::MIN_UPRIGHT_ASPECT;
        let (_, focus) = art::look(&img, None, false, W, ART_H);
        let title = title.trim();
        let meta = ArtMeta {
            title: if title.is_empty() { "无题".into() } else { title.chars().take(60).collect() },
            focus,
            whole,
            source: "我的图库".into(),
            ..ArtMeta::default()
        };
        let key = store::write(&self.admin.store).add_picture(meta, &img).map_err(saving)?;
        Ok(json_reply(200, &json!({ "key": key })))
    }

    fn delete(&self, req: KeyOnly) -> Result<Reply, Fail> {
        if !store::is_own(&req.key) {
            return Err(fail(400, "只能删除自己添加的图"));
        }
        store::write(&self.admin.store).remove_picture(&req.key).map_err(saving)?;
        let _ = std::fs::remove_file(self.thumb_path(&req.key));
        (self.admin.notify)(Change::Rotation);
        Ok(json_reply(200, &json!({ "ok": true })))
    }

    fn render(&self, req: RenderWork) -> Result<Reply, Fail> {
        if !valid_key(&req.key) {
            return Err(fail(400, "画作编号不对"));
        }
        let (mut meta, img) = self.source(&req.key)?;
        if let Some(edit) = &req.edit {
            edit.apply(&mut meta);
        }
        let art = art::prepare(meta, img, W, ART_H);
        let fonts = &self.admin.fonts;
        let mut pages = layout::guide(fonts, &art);
        let total = pages.len();
        let canvas = match req.page {
            0 => layout::clock_c(fonts, &art, &Local::now(), None),
            n => pages.swap_remove(n.min(total) - 1),
        };
        let palette = lock(&self.admin.live).palette.unwrap_or(UNIFORM);
        Ok(reply(200, "image/png", panel_png(&canvas.0, &palette)).with_header(header("X-Pages", &total.to_string())))
    }

    fn thumb_path(&self, key: &str) -> std::path::PathBuf {
        self.admin.cache.dir().join("thumbs").join(format!("{key}.jpg"))
    }

    fn thumb(&self, key: &str) -> Result<Reply, Fail> {
        let path = self.thumb_path(key);
        if let Ok(jpg) = std::fs::read(&path) {
            return Ok(thumb_reply(jpg));
        }
        let img = match self.on_disk(key) {
            Some((_, img)) => img,
            None => {
                let entry = self.entry(key).ok_or_else(|| fail(404, "没有这幅画"))?;
                if self.admin.offline {
                    return Err(fail(503, "离线模式"));
                }
                let small = thumb_url(&entry.image);
                let bytes = match self.client.get(&small, &[], 16 << 20) {
                    Err(_) if small != entry.image => self.client.get(&entry.image, &[], 32 << 20),
                    got => got,
                }
                .map_err(|e| fail(502, e.to_string()))?;
                image::load_from_memory(&bytes).map_err(|e| fail(502, format!("缩略图无法解码：{e}")))?.to_luma8()
            }
        };
        let small = art::fit(&art::tone(img), THUMB_SIDE, THUMB_SIDE);
        let mut jpg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpg, 82)
            .encode_image(&small)
            .map_err(|e| fail(500, e.to_string()))?;
        let _ = write_atomic(&path, &jpg);
        Ok(thumb_reply(jpg))
    }

    /// A built-in, own, kept or cached work, without going to the network.
    fn on_disk(&self, key: &str) -> Option<(ArtMeta, GrayImage)> {
        if let Some((m, img)) = self.builtin(key) {
            return Some((m.clone(), img.clone()));
        }
        let files = store::read(&self.admin.store).files().clone();
        for place in [&files, &self.admin.cache] {
            if place.has_work(key) {
                if let Ok(work) = place.load_work(key) {
                    return Some(work);
                }
            }
        }
        None
    }
}

/// Commons serves smaller copies under the same path with a different width,
/// including "lossy-page1-1280px-" ones made from TIFFs. Links to an original
/// file get the thumbnail path built from it.
fn thumb_url(image: &str) -> String {
    const COMMONS: &str = "/wikipedia/commons/";
    if image.contains("/thumb/") {
        return match image.rfind("1280px-") {
            Some(i) => format!("{}{THUMB_SIDE}px-{}", &image[..i], &image[i + "1280px-".len()..]),
            None => image.to_string(),
        };
    }
    match (image.find(COMMONS), image.rsplit_once('/')) {
        (Some(at), Some((_, name))) if !name.is_empty() => {
            let (host, path) = image.split_at(at + COMMONS.len());
            format!("{host}thumb/{path}/{THUMB_SIDE}px-{name}")
        }
        _ => image.to_string(),
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Phone photos often say how to turn them in EXIF rather than in their pixels.
fn decode_upload(bytes: &[u8]) -> Result<GrayImage, Fail> {
    let unreadable = |e: image::ImageError| fail(400, format!("看不懂这个图片文件（请用 JPEG 或 PNG）：{e}"));
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| fail(400, e.to_string()))?
        .into_decoder()
        .map_err(unreadable)?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder).map_err(unreadable)?;
    img.apply_orientation(orientation);
    Ok(img.to_luma8())
}

fn srgb_to_linear(v: u8) -> f32 {
    let v = v as f32 / 255.0;
    255.0 * if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(l: f32) -> u8 {
    let l = (l / 255.0).clamp(0.0, 1.0);
    let v = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (v * 255.0).round() as u8
}

/// A frame as the panel shows it: dithered to the panel's sixteen greys the way
/// the SDK does it, in linear light with Floyd–Steinberg, then encoded for a screen.
pub fn panel_png(img: &GrayImage, palette: &[u8; 16]) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let mut px: Vec<f32> = img.pixels().map(|p| srgb_to_linear(p.0[0])).collect();
    let shades: Vec<u8> = palette.iter().map(|&l| linear_to_srgb(l as f32)).collect();
    let mut out = GrayImage::new(w, h);
    for row in 0..hu {
        for col in 0..wu {
            let i = row * wu + col;
            let value = px[i].clamp(0.0, 255.0);
            let mut level = 0;
            let mut distance = f32::MAX;
            for (k, &l) in palette.iter().enumerate() {
                let d = (value - l as f32).abs();
                if d < distance {
                    distance = d;
                    level = k;
                }
            }
            let error = value - palette[level] as f32;
            out.put_pixel(col as u32, row as u32, Luma([shades[level]]));
            if col + 1 < wu {
                px[i + 1] += error * 7.0 / 16.0;
            }
            if row + 1 < hu {
                if col > 0 {
                    px[i + wu - 1] += error * 3.0 / 16.0;
                }
                px[i + wu] += error * 5.0 / 16.0;
                if col + 1 < wu {
                    px[i + wu + 1] += error / 16.0;
                }
            }
        }
    }
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(out.as_raw(), w, h, ExtendedColorType::L8)
        .expect("encoding an in-memory grayscale PNG cannot fail");
    png
}

/// Keys go into file names, so only plain ones are accepted.
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("header names and values are ASCII")
}

fn reply(status: u16, content_type: &str, body: Vec<u8>) -> Reply {
    Response::from_data(body)
        .with_status_code(status)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"))
}

fn json_reply(status: u16, value: &Value) -> Reply {
    reply(status, "application/json; charset=utf-8", serde_json::to_vec(value).expect("JSON values serialize"))
}

fn thumb_reply(jpg: Vec<u8>) -> Reply {
    Response::from_data(jpg)
        .with_header(header("Content-Type", "image/jpeg"))
        .with_header(header("Cache-Control", "private, max-age=86400"))
}

fn session_cookie(token: &str) -> Header {
    header("Set-Cookie", &format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=31536000"))
}

fn header_value(req: &Request, name: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn cookie(req: &Request, name: &str) -> Option<String> {
    header_value(req, "Cookie")?.split(';').find_map(|pair| {
        let (k, v) = pair.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// Another site's page cannot send this browser's cookie along with a change.
fn same_origin(req: &Request) -> bool {
    match header_value(req, "Origin") {
        None => true,
        Some(origin) => {
            let host = header_value(req, "Host");
            origin.split_once("://").map(|(_, h)| h.to_string()) == host
        }
    }
}

fn read_body(req: &mut Request, limit: usize) -> Result<Vec<u8>, Fail> {
    if req.body_length().is_some_and(|n| n > limit) {
        return Err(fail(413, format!("内容太大，最多 {} MB", limit >> 20)));
    }
    let mut body = Vec::new();
    req.as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)
        .map_err(|e| fail(400, format!("读取请求失败：{e}")))?;
    if body.len() > limit {
        return Err(fail(413, format!("内容太大，最多 {} MB", limit >> 20)));
    }
    Ok(body)
}

fn json_body<T: DeserializeOwned>(req: &mut Request) -> Result<T, Fail> {
    let body = read_body(req, MAX_JSON)?;
    serde_json::from_slice(&body).map_err(|e| fail(400, format!("请求格式不对：{e}")))
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (percent_decode(k) == name).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_strings_are_decoded() {
        assert_eq!(query_param("key=Q1&title=%E7%8C%AB+%E5%92%AA", "title").as_deref(), Some("猫 咪"));
        assert_eq!(query_param("key=Q1", "key").as_deref(), Some("Q1"));
        assert_eq!(query_param("key=Q1", "title"), None);
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }

    #[test]
    fn thumbnails_come_from_smaller_copies() {
        let base = "https://thumb.wikimedia.org/wikipedia/commons/thumb/f/fa/A.tif/";
        assert_eq!(thumb_url(&format!("{base}lossy-page1-1280px-A.tif.jpg")), format!("{base}lossy-page1-330px-A.tif.jpg"));
        assert_eq!(thumb_url(&format!("{base}1280px-A.jpg")), format!("{base}330px-A.jpg"));
        assert_eq!(
            thumb_url("https://upload.wikimedia.org/wikipedia/commons/8/86/Sengai_3.jpg"),
            "https://upload.wikimedia.org/wikipedia/commons/thumb/8/86/Sengai_3.jpg/330px-Sengai_3.jpg"
        );
        assert_eq!(thumb_url("https://example.org/a.jpg"), "https://example.org/a.jpg");
    }

    #[test]
    fn keys_cannot_leave_their_directory() {
        assert!(valid_key("Q12") && valid_key("my-3") && valid_key("rainy"));
        assert!(!valid_key("../state") && !valid_key("a/b") && !valid_key("") && !valid_key("Q1.json"));
    }

    #[test]
    fn edits_tidy_the_texts() {
        let mut meta = ArtMeta { id: "Q1".into(), title: "星夜".into(), source: "Commons".into(), ..ArtMeta::default() };
        let edit = Edit {
            title: "  ".into(),
            artist: " 文森特·梵高 ".into(),
            intro: "第一段\r\n\r\n  第二段  \n".into(),
            focus: Some(1.7),
            ..Edit::default()
        };
        edit.apply(&mut meta);
        assert_eq!(meta.title, "无题");
        assert_eq!(meta.artist, "文森特·梵高");
        assert_eq!(meta.intro.as_deref(), Some("第一段\n第二段"));
        assert_eq!(meta.museum, None);
        assert_eq!(meta.focus, Some([0.5, 1.0]));
        assert_eq!(meta.source, "Commons", "untouched");
    }

    #[test]
    fn the_preview_uses_only_the_panel_greys() {
        let ramp = GrayImage::from_fn(400, 300, |x, _| Luma([(x * 255 / 399) as u8]));
        let png = panel_png(&ramp, &UNIFORM);
        let shown = image::load_from_memory(&png).unwrap().to_luma8();
        let shades: Vec<u8> = UNIFORM.iter().map(|&l| linear_to_srgb(l as f32)).collect();
        assert!(shown.pixels().all(|p| shades.contains(&p.0[0])));
        // Dithering keeps the average brightness of each column roughly where it was.
        for x in [40u32, 200, 360] {
            let mean = |img: &GrayImage| (0..300).map(|y| srgb_to_linear(img.get_pixel(x, y).0[0])).sum::<f32>() / 300.0;
            assert!((mean(&shown) - mean(&ramp)).abs() < 12.0, "column {x}");
        }
    }
}
