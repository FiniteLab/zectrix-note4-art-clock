//! 画时：NOTE4 名画时钟。
//!
//! 用法：
//!   art-clock <设备 IP> [--cache <目录>] [--data <目录>] [--web <端口>] [--no-web] [--offline]
//!       连接设备，持续显示时钟。联网时按设定的间隔（默认每天）从名画目录（assets/catalog.json）
//!       和自己的图库里换画，名画从 Wikimedia Commons 下载；离线时用内置的画。目录由 catalog 工具生成。
//!       同时在 8686 端口开管理网页：设置轮播或固定一幅、换画间隔、名画和自己图的比例，
//!       上传自己的图，改标题和讲解。
//!       按键：确认 = 打开 / 关闭画作介绍；上 / 下 = 换画（介绍里是翻页）；
//!       长按上或下 = 把屏幕交还给设备，之后在设备上选「设置 → 网络同步 → 连接管理端」回来。
//!   art-clock --render <输出.png> [--at "2026-11-04 10:24"] [--art <序号>] [--guide <页>] [--demo]
//!       不连设备，只渲染一帧 PNG。--art 是画廊里的序号（0 = 当前的画），
//!       --guide 渲染介绍的第几页（从 1 开始），--demo 填入示例电量和 Wi-Fi。
//!   art-clock --sync [--days <N>]
//!       只取画：当前时段、下一个时段和之前 N 个时段（默认 6），然后退出。
//! 选项：
//!   --assets <目录>   素材目录，默认是源码里的 assets/
//!   --cache <目录>    画作缓存，默认 ~/.cache/art-clock
//!   --data <目录>     设置、自己的图和改过的讲解，默认 ~/.local/share/art-clock
//!   --web <端口>      管理网页的端口（或 地址:端口），默认 8686；--no-web 不开
//!   --offline         不联网取画，只用内置的画和已缓存的画
use chrono::{DateTime, Local, NaiveDateTime, TimeDelta, Timelike};
use image::GrayImage;
use note4_art_clock::{
    art::{self, ArtMeta, Artwork},
    catalog::{self, Entry},
    layout::{self, Fonts, Status},
    library::{self, Cache, State, Syncer},
    render::{Canvas, W},
    store::{self, Mode, Settings, Store},
    web::{self, Admin, Change, Live},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc, Mutex, PoisonError, RwLock},
    time::Duration,
};
use tokio::{
    signal::unix::{signal, Signal, SignalKind},
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::Instant,
};
use zectrix_sdk::{
    ConnectionRequest, ConnectionRequestListener, Device, DisconnectReason, Error, Event,
    ImageFormat, Key, KeyAction, OpenOptions,
};

const CLIENT_NAME: &str = "画时 Art Clock";
/// Measured on the device: a gray16 frame takes about 1.9 s from push to refresh done.
const INITIAL_LEAD: Duration = Duration::from_millis(1900);
/// Battery and Wi-Fi change slowly; the SDK asks for at least 30 s between queries.
const STATUS_EVERY: Duration = Duration::from_secs(5 * 60);
const STATUS_MIN_GAP: Duration = Duration::from_secs(30);
/// The guide goes back to the clock after this long without a key press.
const GUIDE_IDLE: Duration = Duration::from_secs(2 * 60);
const SYNC_EVERY: Duration = Duration::from_secs(60 * 60);
const SYNC_RETRY: Duration = Duration::from_secs(15 * 60);
/// The sync wakes this long after a slot starts, to fetch the one after it.
const SYNC_AFTER_SLOT: Duration = Duration::from_secs(2);
const USAGE: &str = "\
用法：art-clock <设备 IP> [--cache <目录>] [--data <目录>] [--web <端口>] [--no-web] [--offline] [--assets <目录>]
      art-clock --render <输出.png> [--at \"YYYY-MM-DD HH:MM\"] [--art <序号>] [--guide <页>] [--demo]
      art-clock --sync [--days <N>]";

macro_rules! log {
    ($($arg:tt)*) => {
        println!("[{}] {}", Local::now().format("%H:%M:%S%.3f"), format!($($arg)*))
    };
}

struct Args {
    target: Option<String>,
    assets: PathBuf,
    cache: Option<PathBuf>,
    data: Option<PathBuf>,
    web: Option<String>,
    offline: bool,
    sync: bool,
    days: i64,
    render: Option<PathBuf>,
    at: Option<NaiveDateTime>,
    art: Option<usize>,
    guide: Option<usize>,
    demo: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        target: None,
        assets: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        cache: None,
        data: None,
        web: Some(format!("0.0.0.0:{}", web::DEFAULT_PORT)),
        offline: false,
        sync: false,
        days: library::BEHIND,
        render: None,
        at: None,
        art: None,
        guide: None,
        demo: false,
    };
    let mut it = std::env::args().skip(1);
    let number = |it: &mut dyn Iterator<Item = String>, flag: &str| -> Result<usize, String> {
        let s = it.next().ok_or(format!("{flag} 需要数字"))?;
        s.parse().map_err(|_| format!("{flag} 需要数字，收到 {s}"))
    };
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--assets" => args.assets = it.next().ok_or("--assets 需要目录")?.into(),
            "--cache" => args.cache = Some(it.next().ok_or("--cache 需要目录")?.into()),
            "--data" => args.data = Some(it.next().ok_or("--data 需要目录")?.into()),
            "--web" => {
                let s = it.next().ok_or("--web 需要端口，例如 8686")?;
                args.web = Some(if s.parse::<u16>().is_ok() { format!("0.0.0.0:{s}") } else { s });
            }
            "--no-web" => args.web = None,
            "--offline" => args.offline = true,
            "--sync" => args.sync = true,
            "--days" => args.days = number(&mut it, "--days")? as i64,
            "--render" => args.render = Some(it.next().ok_or("--render 需要输出文件")?.into()),
            "--at" => {
                let s = it.next().ok_or("--at 需要时间，例如 \"2026-11-04 10:24\"")?;
                args.at = Some(
                    NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M")
                        .map_err(|e| format!("--at 格式应为 \"YYYY-MM-DD HH:MM\"：{e}"))?,
                );
            }
            "--art" => args.art = Some(number(&mut it, "--art")?),
            "--guide" => args.guide = Some(number(&mut it, "--guide")?),
            "--demo" => args.demo = true,
            "-h" | "--help" => return Err(USAGE.into()),
            s if s.starts_with('-') => return Err(format!("未知选项 {s}\n{USAGE}")),
            _ => args.target = Some(arg),
        }
    }
    if args.render.is_none() && args.target.is_none() && !args.sync {
        return Err(USAGE.into());
    }
    Ok(args)
}

enum Screen {
    Clock,
    Guide {
        pages: Vec<Vec<u8>>,
        page: usize,
        touched: Instant,
    },
}

/// Why the clock should look again at what to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Update {
    /// The syncer saved a work.
    Works,
    /// Settings or texts changed on the admin page: reload everything.
    Changed,
}

struct App {
    fonts: Arc<Fonts>,
    /// The built-in pictures as loaded, before their texts are edited and toned.
    sources: Arc<Vec<(ArtMeta, GrayImage)>>,
    builtins: Vec<Artwork>,
    cache: Cache,
    store: Arc<RwLock<Store>>,
    live: Arc<Mutex<Live>>,
    /// Which work fills which slot, as last read from the cache.
    state: State,
    settings: Settings,
    works: HashMap<String, Artwork>,
    /// Up/Down steps away from the slot's picture; reset when the slot changes.
    browse: i64,
    browse_slot: Option<NaiveDateTime>,
    screen: Screen,
    lead: Duration,
    status: Option<Status>,
    status_at: Option<Instant>,
    /// The frame on screen, so updates that change nothing are not pushed again.
    last_frame: Vec<u8>,
}

impl App {
    /// Picks up works the syncer saved since the last call and drops expired ones;
    /// `full` also applies new settings and edited texts to everything.
    fn reload(&mut self, full: bool) {
        let settings = store::read(&self.store).settings().clone();
        if full || self.builtins.is_empty() {
            self.works.clear();
            let store = store::read(&self.store);
            self.builtins = self
                .sources
                .iter()
                .map(|(meta, img)| art::prepare(store.apply(meta.clone()), img.clone(), W, layout::ART_H))
                .collect();
        }
        if settings != self.settings {
            self.browse = 0;
        }
        let state = self.cache.state();
        let wanted: Vec<String> = state
            .slots
            .values()
            .map(String::as_str)
            .chain(settings.fixed_key())
            .filter(|k| !self.builtins.iter().any(|b| b.meta.id == *k))
            .map(str::to_string)
            .collect();
        for key in &wanted {
            if self.works.contains_key(key) {
                continue;
            }
            match self.load(key) {
                Ok(work) => {
                    self.works.insert(key.clone(), work);
                }
                Err(e) => log!("载入 {key} 失败：{e}"),
            }
        }
        self.works.retain(|k, _| wanted.contains(k));
        self.state = state;
        self.settings = settings;
    }

    /// Own pictures and the fixed work live in the data directory, the rest in the cache.
    fn load(&self, key: &str) -> Result<Artwork, String> {
        let store = store::read(&self.store);
        let (meta, img) = if store.files().has_work(key) {
            store.files().load_work(key)?
        } else {
            self.cache.load_work(key)?
        };
        Ok(art::prepare(store.apply(meta), img, W, layout::ART_H))
    }

    fn roll(&mut self, t: NaiveDateTime) {
        let slot = library::slot_start(t, self.settings.interval);
        if self.browse_slot != Some(slot) {
            self.browse_slot = Some(slot);
            self.browse = 0;
        }
    }

    /// Up/Down order: the fixed picture or the slot's, earlier slots newest first,
    /// then the built-ins, which lead when nothing has been downloaded.
    fn gallery(&self, t: NaiveDateTime) -> Vec<&Artwork> {
        let builtin = |key: &str| self.builtins.iter().find(|b| b.meta.id == key);
        let mut out: Vec<&Artwork> = library::lineup(&self.state, &self.settings, t)
            .iter()
            .filter_map(|k| self.works.get(k).or_else(|| builtin(k)))
            .collect();
        let n = self.builtins.len();
        let interval = self.settings.interval;
        let first = library::slot_number(library::slot_start(t, interval), interval).rem_euclid(n as i64) as usize;
        for i in 0..n {
            let b = &self.builtins[(first + i) % n];
            if !out.iter().any(|a| a.meta.id == b.meta.id) {
                out.push(b);
            }
        }
        out
    }

    fn art(&self, t: &DateTime<Local>) -> &Artwork {
        let gallery = self.gallery(t.naive_local());
        gallery[self.browse.rem_euclid(gallery.len() as i64) as usize]
    }

    fn frame(&mut self, t: &DateTime<Local>) -> Vec<u8> {
        self.roll(t.naive_local());
        layout::clock_c(&self.fonts, self.art(t), t, self.status.as_ref()).to_png()
    }

    fn open_guide(&mut self, t: &DateTime<Local>) {
        self.roll(t.naive_local());
        let pages = layout::guide(&self.fonts, self.art(t)).iter().map(Canvas::to_png).collect();
        self.screen = Screen::Guide { pages, page: 0, touched: Instant::now() };
    }

    fn status_due(&self, gap: Duration) -> bool {
        self.status_at.is_none_or(|at| at.elapsed() >= gap)
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[tokio::main]
async fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    if let Err(e) = run(args).await {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let cache_dir = args.cache.clone().unwrap_or_else(Cache::default_dir);
    let data_dir = args.data.clone().unwrap_or_else(Store::default_dir);
    let catalog_path = args.assets.join("catalog.json");
    let store = Arc::new(RwLock::new(Store::open(data_dir.clone())));
    if args.sync {
        log!("画作缓存：{}", cache_dir.display());
        let catalog = Arc::new(catalog::load(&catalog_path)?);
        let mut syncer = Syncer::new(Cache::new(cache_dir), catalog, store);
        syncer.sync(Local::now().naive_local(), args.days, &mut |m| log!("{m}"), &mut || {})?;
        log!("取画完成");
        return Ok(());
    }

    let fonts = Arc::new(Fonts::load(&args.assets.join("fonts"))?);
    let sources = Arc::new(art::load_builtins(&args.assets.join("art"))?);
    let live = Arc::new(Mutex::new(Live::default()));
    let mut app = App {
        fonts: fonts.clone(),
        sources: sources.clone(),
        builtins: Vec::new(),
        cache: Cache::new(cache_dir.clone()),
        store: store.clone(),
        live: live.clone(),
        state: State::default(),
        settings: store::read(&store).settings().clone(),
        works: HashMap::new(),
        browse: 0,
        browse_slot: None,
        screen: Screen::Clock,
        lead: INITIAL_LEAD,
        status: None,
        status_at: None,
        last_frame: Vec::new(),
    };
    app.reload(true);

    if let Some(out) = args.render {
        let t = match args.at {
            Some(at) => at
                .and_local_timezone(Local)
                .single()
                .ok_or("这个本地时间不存在或有歧义（夏令时切换）")?,
            None => Local::now(),
        };
        app.roll(t.naive_local());
        app.browse = args.art.unwrap_or(0) as i64;
        if args.demo {
            app.status = Some(Status {
                battery_percent: Some(80),
                charging: false,
                wifi_rssi_dbm: Some(-52),
            });
        }
        let png = match args.guide {
            Some(n) => {
                app.open_guide(&t);
                let Screen::Guide { pages, .. } = &app.screen else {
                    unreachable!("open_guide always opens the guide")
                };
                let i = n
                    .checked_sub(1)
                    .filter(|&i| i < pages.len())
                    .ok_or(format!("--guide 页码应在 1 到 {} 之间", pages.len()))?;
                pages[i].clone()
            }
            None => app.frame(&t),
        };
        std::fs::write(&out, png)?;
        let total = app.gallery(t.naive_local()).len();
        println!(
            "已渲染 {}：{}，{}（画廊第 {}/{total} 幅）",
            out.display(),
            t.format("%Y-%m-%d %H:%M"),
            app.art(&t).meta.title,
            app.browse.rem_euclid(total as i64) + 1,
        );
        return Ok(());
    }

    let target = args.target.expect("parse_args requires a target when not rendering or syncing");
    log!("画作缓存：{}，已缓存 {} 幅；我的数据：{}", cache_dir.display(), app.works.len(), data_dir.display());
    let catalog = if args.offline {
        log!("离线模式：不取新画");
        None
    } else {
        match catalog::load(&catalog_path) {
            Ok(list) => {
                log!("名画目录 {} 幅", list.len());
                Some(Arc::new(list))
            }
            Err(e) => {
                log!("名画目录读不出来（{e}），只用内置和已缓存的画");
                None
            }
        }
    };
    let (ui_tx, ui_rx) = unbounded_channel();
    let sync = catalog
        .clone()
        .map(|list| start_sync(Cache::new(cache_dir.clone()), list, store.clone(), ui_tx.clone()));
    if let Some(addr) = &args.web {
        let notify = {
            let store = store.clone();
            let ui = ui_tx.clone();
            let sync = sync.clone();
            move |change: Change| {
                let fixed = store::read(&store).settings().mode == Mode::Fixed;
                match &sync {
                    // The syncer fills the slot again first, so the screen changes once.
                    Some(tx) if change == Change::Rotation => {
                        let _ = tx.send(());
                        if fixed {
                            let _ = ui.send(Update::Changed);
                        }
                    }
                    _ => {
                        let _ = ui.send(Update::Changed);
                    }
                }
            }
        };
        let admin = Admin {
            store: store.clone(),
            cache: Cache::new(cache_dir.clone()),
            catalog: catalog.clone().unwrap_or_default(),
            builtins: sources.clone(),
            fonts: fonts.clone(),
            live: live.clone(),
            notify: Box::new(notify),
            offline: args.offline,
        };
        match web::serve(admin, addr) {
            Ok(()) => {
                let port = addr.rsplit(':').next().unwrap_or("8686");
                let host = local_ip().unwrap_or_else(|| "本机 IP".into());
                log!("管理网页：http://{host}:{port}/");
            }
            Err(e) => log!("{e}"),
        }
    }
    drop(ui_tx);
    clock(&mut app, &target, Some(ui_rx)).await
}

/// The address other machines on the LAN reach this one at. Connecting a UDP
/// socket sends nothing; it only picks the outgoing interface.
fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:53").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

/// Fetches pictures on a plain thread, since the HTTP client blocks. It wakes
/// when a slot starts, to fetch the next one, and when the admin page changes how
/// pictures take turns.
fn start_sync(
    cache: Cache,
    catalog: Arc<Vec<Entry>>,
    store: Arc<RwLock<Store>>,
    ui: UnboundedSender<Update>,
) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("sync".into())
        .spawn(move || {
            let mut syncer = Syncer::new(cache, catalog, store.clone());
            let mut reset = false;
            loop {
                let now = Local::now().naive_local();
                if reset {
                    if let Err(e) = syncer.reset(now) {
                        log!("取画：{e}");
                    }
                }
                let mut ready = || {
                    let _ = ui.send(Update::Works);
                };
                let result = syncer.sync(now, library::BEHIND, &mut |m| log!("取画：{m}"), &mut ready);
                if reset {
                    let _ = ui.send(Update::Changed);
                }
                let wait = match result {
                    Ok(()) => {
                        let interval = store::read(&store).settings().interval;
                        let next = library::next_slot(library::slot_start(now, interval), interval);
                        let until = (next - Local::now().naive_local()).to_std().unwrap_or(Duration::ZERO);
                        (until + SYNC_AFTER_SLOT).min(SYNC_EVERY)
                    }
                    Err(e) => {
                        log!("取画暂停：{e}，{} 分钟后再试", SYNC_RETRY.as_secs() / 60);
                        SYNC_RETRY
                    }
                };
                if ui.is_closed() {
                    return;
                }
                match rx.recv_timeout(wait) {
                    Ok(()) => {
                        while rx.try_recv().is_ok() {}
                        reset = true;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => reset = false,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("starting the sync thread");
    tx
}
enum Lost {
    /// The user left remote mode on the device; wait for the device to call back.
    DeviceExit,
    /// A long press on Up or Down: give the screen back and wait for the device.
    HandBack,
    Retry,
}

async fn clock(
    app: &mut App,
    target: &str,
    mut updates: Option<UnboundedReceiver<Update>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut listener = match ConnectionRequestListener::bind().await {
        Ok(l) => Some(l),
        Err(e) => {
            log!("无法监听设备发起的连接（UDP 32343）：{e}");
            None
        }
    };
    let mut term = signal(SignalKind::terminate())?;
    let mut wait_for_device = false;
    loop {
        let Some(device) = connect(target, &mut listener, wait_for_device, &mut term).await else {
            log!("程序退出");
            return Ok(());
        };
        log!("已连接 {}（{}）", device.summary.name, device.summary.address);
        {
            let mut live = app.live();
            live.connected = true;
            live.palette = Some(device.palette().gray16);
        }
        let outcome = drive(app, &device, &mut listener, &mut updates, &mut term).await;
        let _ = device.close().await;
        {
            let mut live = app.live();
            live.connected = false;
            live.showing = None;
        }
        match outcome {
            Some(lost) => wait_for_device = matches!(lost, Lost::DeviceExit | Lost::HandBack),
            None => {
                log!("已断开，程序退出");
                return Ok(());
            }
        }
    }
}

/// Runs the clock on one session. `None` means shutdown was requested.
async fn drive(
    app: &mut App,
    device: &Device,
    listener: &mut Option<ConnectionRequestListener>,
    updates: &mut Option<UnboundedReceiver<Update>>,
    term: &mut Signal,
) -> Option<Lost> {
    app.reload(false);
    app.screen = Screen::Clock;
    if app.status_due(STATUS_MIN_GAP) {
        if let Err(lost) = query_status(app, device).await {
            return Some(lost);
        }
    }
    let mut shown = minute_floor(Local::now());
    // The first frame of a session is slow (Wi-Fi wake-up, palette setup), so it does
    // not train the lead.
    if let Err(lost) = push(app, device, &shown, false).await {
        return Some(lost);
    }
    loop {
        let next = next_minute(shown, Local::now(), app.lead);
        let push_at = next - TimeDelta::from_std(app.lead).unwrap_or(TimeDelta::zero());
        let wait = (push_at - Local::now()).to_std().unwrap_or(Duration::ZERO);
        let deadline = Instant::now() + wait;
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                let guide_idle = match &app.screen {
                    Screen::Clock => None,
                    Screen::Guide { touched, .. } => Some(touched.elapsed() >= GUIDE_IDLE),
                };
                if guide_idle == Some(false) {
                    // The guide stays up; the clock catches up when it closes.
                    shown = next;
                } else {
                    if guide_idle == Some(true) {
                        log!("介绍页 {} 分钟没有操作，回到时钟", GUIDE_IDLE.as_secs() / 60);
                        app.screen = Screen::Clock;
                    }
                    if let Err(lost) = push(app, device, &next, true).await {
                        return Some(lost);
                    }
                    shown = next;
                    // Right after a push the link is idle for most of a minute.
                    if app.status_due(STATUS_EVERY) {
                        if let Err(lost) = query_status(app, device).await {
                            return Some(lost);
                        }
                    }
                }
            }
            event = device.next_event() => match event {
                Ok(Some(Event::Key { key, action })) => {
                    if let Err(lost) = on_key(app, device, key, action, &mut shown).await {
                        return Some(lost);
                    }
                }
                Ok(Some(Event::Disconnected { reason })) => {
                    log!("连接结束：{reason:?}");
                    return Some(if reason == DisconnectReason::DeviceExit { Lost::DeviceExit } else { Lost::Retry });
                }
                Ok(Some(_)) => {}
                Ok(None) => return Some(Lost::Retry),
                Err(e) => {
                    log!("事件错误：{e}");
                    return Some(Lost::Retry);
                }
            },
            Some(update) = next_update(updates) => {
                app.reload(update == Update::Changed);
                if matches!(app.screen, Screen::Clock) {
                    let png = app.frame(&shown);
                    if png != app.last_frame {
                        log!("画面有变：{}", app.art(&shown).meta.title);
                        if let Err(lost) = push_frame(app, device, &shown, png, false).await {
                            return Some(lost);
                        }
                    }
                }
            }
            Some(req) = next_request(listener) => {
                log!("已在连接中，忽略来自 {} 的连接请求", req.address);
            }
            _ = tokio::signal::ctrl_c() => return None,
            _ = term.recv() => return None,
        }
    }
}

enum Step {
    Ignore,
    HandBack,
    Browse(i64),
    OpenGuide,
    Page(usize),
    CloseGuide,
}

async fn on_key(
    app: &mut App,
    device: &Device,
    key: Key,
    action: KeyAction,
    shown: &mut DateTime<Local>,
) -> Result<(), Lost> {
    let step = match (&app.screen, key, action) {
        (_, Key::Up | Key::Down, KeyAction::Long) => Step::HandBack,
        // A long Confirm is the device's own voice recorder.
        (_, _, KeyAction::Long) => Step::Ignore,
        (Screen::Clock, Key::Confirm, _) => Step::OpenGuide,
        (Screen::Clock, Key::Up, _) => Step::Browse(-1),
        (Screen::Clock, Key::Down, _) => Step::Browse(1),
        (Screen::Guide { .. }, Key::Confirm, _) => Step::CloseGuide,
        (Screen::Guide { page, pages, .. }, Key::Down, _) if page + 1 < pages.len() => {
            Step::Page(page + 1)
        }
        (Screen::Guide { page, .. }, Key::Up, _) if *page > 0 => Step::Page(page - 1),
        (Screen::Guide { .. }, _, _) => Step::CloseGuide,
    };
    match step {
        Step::Ignore => Ok(()),
        Step::HandBack => {
            log!("长按{}：把屏幕交还给设备", if key == Key::Up { "上键" } else { "下键" });
            Err(Lost::HandBack)
        }
        Step::Browse(by) => {
            app.roll(shown.naive_local());
            app.browse += by;
            log!("换画：{}", app.art(shown).meta.title);
            push(app, device, shown, true).await
        }
        Step::OpenGuide => {
            app.open_guide(shown);
            log!("打开介绍：{}", app.art(shown).meta.title);
            show_guide(app, device).await
        }
        Step::Page(p) => {
            if let Screen::Guide { page, touched, .. } = &mut app.screen {
                *page = p;
                *touched = Instant::now();
            }
            show_guide(app, device).await
        }
        Step::CloseGuide => {
            app.screen = Screen::Clock;
            *shown = minute_floor(Local::now());
            push(app, device, shown, false).await
        }
    }
}

async fn show_guide(app: &App, device: &Device) -> Result<(), Lost> {
    let Screen::Guide { pages, page, .. } = &app.screen else {
        return Ok(());
    };
    let started = Instant::now();
    match device.show_with_format(&pages[*page], ImageFormat::Gray16).await {
        Ok(()) => {
            log!(
                "介绍第 {}/{} 页，用时 {:.2}s",
                page + 1,
                pages.len(),
                started.elapsed().as_secs_f64()
            );
            Ok(())
        }
        Err(Error::Disconnected) => Err(Lost::Retry),
        Err(e) => {
            log!("介绍页显示失败：{e}");
            Ok(())
        }
    }
}

/// A failed query keeps the last reading on screen; only a lost connection is fatal.
async fn query_status(app: &mut App, device: &Device) -> Result<(), Lost> {
    app.status_at = Some(Instant::now());
    match device.status().await {
        Ok(s) => {
            let status = Status {
                battery_percent: s.battery_percent,
                charging: s.charging.unwrap_or(false),
                wifi_rssi_dbm: s.wifi_rssi_dbm,
            };
            log!(
                "设备状态：电量 {}{}，Wi-Fi {}",
                status.battery_percent.map_or("未知".into(), |p| format!("{p}%")),
                if status.charging { "（充电中）" } else { "" },
                status.wifi_rssi_dbm.map_or("未知".into(), |r| format!("{r} dBm")),
            );
            app.status = Some(status);
            Ok(())
        }
        Err(Error::Disconnected) => Err(Lost::Retry),
        Err(e) => {
            log!("读取设备状态失败：{e}");
            Ok(())
        }
    }
}

async fn push(app: &mut App, device: &Device, t: &DateTime<Local>, learn: bool) -> Result<(), Lost> {
    let png = app.frame(t);
    push_frame(app, device, t, png, learn).await
}

async fn push_frame(
    app: &mut App,
    device: &Device,
    t: &DateTime<Local>,
    png: Vec<u8>,
    learn: bool,
) -> Result<(), Lost> {
    let started = Instant::now();
    match device.show_with_format(&png, ImageFormat::Gray16).await {
        Ok(()) => {
            app.last_frame = png;
            let key = app.art(t).meta.id.clone();
            app.live().showing = Some(key);
            let took = started.elapsed().as_secs_f64();
            if learn {
                // Smooth the lead, and cap each sample, so one stalled refresh does not
                // make the following minutes flip early.
                let lead = app.lead.as_secs_f64();
                let sample = took.min(lead + 1.0);
                app.lead = Duration::from_secs_f64((lead * 0.7 + sample * 0.3).clamp(0.5, 6.0));
            }
            log!(
                "显示 {}，用时 {took:.2}s，提前量 {:.2}s",
                t.format("%H:%M"),
                app.lead.as_secs_f64()
            );
            Ok(())
        }
        Err(Error::Disconnected) => {
            log!("显示 {} 时连接已断开", t.format("%H:%M"));
            Err(Lost::Retry)
        }
        // ResultUnknown must not be resent automatically; the next minute replaces it anyway.
        Err(e) => {
            log!("显示 {} 失败：{e}，等下一分钟", t.format("%H:%M"));
            Ok(())
        }
    }
}

async fn connect(
    target: &str,
    listener: &mut Option<ConnectionRequestListener>,
    wait_for_device: bool,
    term: &mut Signal,
) -> Option<Device> {
    if wait_for_device {
        log!("屏幕已交还设备。要恢复时钟，在设备上选择「设置 → 网络同步 → 连接管理端」");
    }
    // Without a listener the device cannot call back, so fall back to slow polling.
    let poll = !wait_for_device || listener.is_none();
    let mut retry_in = if wait_for_device { Duration::from_secs(60) } else { Duration::ZERO };
    loop {
        tokio::select! {
            _ = tokio::time::sleep(retry_in), if poll => {
                let opened = Device::open(OpenOptions {
                    target: Some(target.to_string()),
                    client_name: Some(CLIENT_NAME.into()),
                })
                .await;
                match opened {
                    Ok(device) => return Some(device),
                    Err(e) => {
                        retry_in = (retry_in * 2).clamp(Duration::from_secs(3), Duration::from_secs(30));
                        log!("连接 {target} 失败：{e}，{} 秒后重试", retry_in.as_secs());
                    }
                }
            }
            Some(req) = next_request(listener) => {
                log!("收到设备发起的连接（{}）", req.address);
                match req.accept(CLIENT_NAME).await {
                    Ok(device) => return Some(device),
                    Err(e) => log!("接受连接失败：{e}"),
                }
            }
            _ = tokio::signal::ctrl_c() => return None,
            _ = term.recv() => return None,
        }
    }
}

async fn next_request(listener: &mut Option<ConnectionRequestListener>) -> Option<ConnectionRequest> {
    match listener {
        Some(l) => match l.next_request().await {
            Ok(req) => Some(req),
            Err(e) => {
                log!("连接请求监听出错：{e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                None
            }
        },
        None => std::future::pending().await,
    }
}

/// Waits for news from the syncer or the admin page; never returns once both have stopped.
async fn next_update(updates: &mut Option<UnboundedReceiver<Update>>) -> Option<Update> {
    let Some(rx) = updates else {
        return std::future::pending().await;
    };
    if let Some(update) = rx.recv().await {
        return Some(update);
    }
    *updates = None;
    std::future::pending().await
}

fn minute_floor(t: DateTime<Local>) -> DateTime<Local> {
    t.with_second(0).and_then(|t| t.with_nanosecond(0)).unwrap_or(t)
}

/// The next minute to put on screen. If pushes fell behind (sleep, slow network),
/// jump to the minute that will be current once a push started now completes.
fn next_minute(shown: DateTime<Local>, now: DateTime<Local>, lead: Duration) -> DateTime<Local> {
    let lead = TimeDelta::from_std(lead).unwrap_or(TimeDelta::zero());
    let earliest = minute_floor(now + lead);
    let next = shown + TimeDelta::minutes(1);
    next.max(earliest)
}
