//! 生成名画目录 assets/catalog.json。在开发机上运行，需要能连上 QLever、Wikidata、
//! Wikimedia Commons 和中文维基百科；时钟本身只读生成的目录。
//!
//! 用法：catalog [--assets <目录>] [--work <目录>]
//!   --assets  素材目录，默认是源码里的 assets/：读 catalog-review.json，写 catalog.json
//!   --work    下载的小图、评审用的横幅图和报告，默认 ~/.cache/art-clock-catalog
//!
//! 候选分四组：有中文维基条目的名作；没有中文条目、但有中文名并在多种语言的维基百科里
//! 都有条目的作品；北斋和广重的几套名所版画；几位名家最知名的版画。依次筛掉：Commons 上
//! 没有公有领域的图、形状不合适、作者还在版权期内、裸体题材、绢本、竖幅却不是风景、
//! 没有中文标题的、评审时剔除的、墨水屏上发灰的、和别的作品是同一幅图的、作品和作者
//! 都没有中文维基条目因而没有讲解的。
use image::{imageops, GrayImage};
use note4_art_clock::{
    art::{self, ArtMeta, Fit, Shape},
    catalog::Entry,
    layout::ART_H,
    library::FAMOUS,
    net::{Client, FetchError},
    render::W,
    wiki::{self, Article, ImageInfo},
    zh,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Duration,
};

const PREFIXES: &str = "PREFIX wd: <http://www.wikidata.org/entity/>
PREFIX wdt: <http://www.wikidata.org/prop/direct/>
PREFIX wikibase: <http://wikiba.se/ontology#>
PREFIX schema: <http://schema.org/>
PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>";
/// Painting, drawing, print, woodblock print, woodcut, etching.
const FAMOUS_CLASSES: &str = "wd:Q3305213 wd:Q93184 wd:Q11060274 wd:Q28913685 wd:Q18219090 wd:Q18218093";
/// Print, woodblock print, woodcut, etching, engraving, lithograph.
const PRINT_CLASSES: &str = "wd:Q11060274 wd:Q28913685 wd:Q18219090 wd:Q18218093 wd:Q11835431 wd:Q15123870";
/// Print series by Hokusai and Hiroshige, with the names shown for them. They are
/// views of places, so their upright prints are kept too.
const SERIES: &[(&str, &str)] = &[
    ("Q227494", "富岳三十六景"),
    ("Q243248", "东海道五十三次"),
    ("Q165190", "名所江户百景"),
    ("Q2963437", "千绘之海"),
    ("Q61040815", "琉球八景"),
    ("Q60555241", "诸国名桥奇览"),
    ("Q75211901", "京都名所"),
    ("Q96305535", "江户高名会亭尽"),
    ("Q60533839", "百人一首乳母绘解"),
];
/// How the series' names start the titles of their prints, besides the names above.
const TITLE_PREFIXES: &[&str] = &["东海道五拾三次之内", "东海道五十三次之内", "东海道五拾三次", "江户名所百景", "百人一首姥之绘解", "百人一首姥之绘说"];
/// Hokusai, Hiroshige, Rembrandt, Dürer, Piranesi, Canaletto, Whistler, Turner,
/// Hogarth and Doré, whose prints read well in black and white.
const PRINTMAKERS: &str = "wd:Q5586 wd:Q200798 wd:Q5598 wd:Q5580 wd:Q316307 wd:Q182664 wd:Q203643 wd:Q159758 wd:Q171344 wd:Q6682";
/// Wikimedia sites with a page on the work (Wikipedias, Commons...). Museums list
/// thousands of prints by these artists; only the ones written about elsewhere count.
const PRINT_MIN_SITELINKS: u32 = 3;
/// For works with no Chinese article: written about elsewhere, and named in
/// Chinese, or in Japanese kanji.
const KNOWN_MIN_SITELINKS: u32 = 3;
const NAME_LANGS: &str = "\"zh\", \"zh-cn\", \"zh-hans\", \"zh-hant\", \"zh-tw\", \"zh-hk\", \"ja\"";
/// Public domain where the term is longest, the creator's life plus 70 years.
const DIED_BY: i64 = 1955;
/// For works with no known creator.
const MADE_BY: i64 = 1900;
const ANONYMOUS_QID: &str = "Q4233718";
const ANONYMOUS: &str = "佚名";
const INTRO_MAX: usize = 700;
const LANGS: &str = "zh-cn|zh-hans|zh|zh-hant|zh-tw|zh-hk|en|ja";
const SCORE_THREADS: usize = 4;
/// Widths Commons keeps ready. Scoring takes the smallest whose copy is still
/// tall enough for the size check, which the clock's own download passes too.
const SCORE_WIDTHS: [u32; 3] = [500, 960, wiki::IMAGE_WIDTH];
const MIN_SIDE: f32 = 300.0;
/// Mean difference, out of 255, between two pictures' 16×16 thumbnails below which
/// they are the same picture, e.g. one print catalogued by two museums. Different
/// works of a kind, such as text plates or sparse line sketches, come out from 7 up.
const DUPLICATE_DIFF: f32 = 6.0;
const TARGET: usize = 365;

#[derive(Clone, PartialEq)]
enum Group {
    /// Has its own Chinese Wikipedia article.
    Famous,
    /// Has a Chinese name and articles in other languages.
    Known,
    Series(String),
    Prints(String),
}

impl Group {
    fn rank(&self) -> u8 {
        match self {
            Group::Famous => 0,
            Group::Known => 1,
            Group::Series(_) => 2,
            Group::Prints(_) => 3,
        }
    }
}

const GROUP_NAMES: [&str; 4] = ["名作", "知名作品", "系列", "版画"];

fn series_name(qid: &str) -> &'static str {
    SERIES.iter().find(|(q, _)| *q == qid).map_or("版画系列", |(_, name)| name)
}

/// Thumbnail used to spot the same picture under two entries.
type Signature = [u8; 256];

#[derive(Clone)]
struct Candidate {
    qid: String,
    group: Group,
    file: String,
    fame: u32,
    /// The work's own Chinese Wikipedia article.
    article: Option<String>,
}

#[derive(Default, Deserialize)]
struct Review {
    /// Works taken out after looking at them, with the reason.
    #[serde(default)]
    exclude: BTreeMap<String, String>,
    /// Hand-picked crops, 0..1 on each axis.
    #[serde(default)]
    focus: BTreeMap<String, [f32; 2]>,
    /// Chinese titles for works that have none on Wikidata.
    #[serde(default)]
    title: BTreeMap<String, String>,
}

/// Labels and details of the items the works point to: creators, series, genres...
struct Refs {
    full: HashMap<String, Value>,
    labels: HashMap<String, Value>,
}

impl Refs {
    fn get(&self, id: &str) -> Option<&Value> {
        self.full.get(id).or_else(|| self.labels.get(id))
    }

    fn zh(&self, id: &str) -> Option<String> {
        self.get(id).and_then(|e| wiki::zh_text(&e["labels"]))
    }

    fn en(&self, id: &str) -> Option<String> {
        self.get(id).and_then(|e| wiki::lang_text(&e["labels"], "en"))
    }

    fn zh_list(&self, ids: &[String], max: usize) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in ids.iter().filter_map(|id| self.zh(id)) {
            if !out.contains(&name) && out.len() < max {
                out.push(name);
            }
        }
        out
    }

    fn zhwiki(&self, id: &str) -> Option<String> {
        self.full.get(id)?["sitelinks"]["zhwiki"]["title"].as_str().map(str::to_string)
    }
}

struct Args {
    assets: PathBuf,
    work: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        assets: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets"),
        work: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".cache/art-clock-catalog"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--assets" => args.assets = it.next().ok_or("--assets 需要目录")?.into(),
            "--work" => args.work = it.next().ok_or("--work 需要目录")?.into(),
            _ => return Err("用法：catalog [--assets <目录>] [--work <目录>]".into()),
        }
    }
    Ok(args)
}

fn main() {
    if let Err(e) = parse_args().and_then(|a| run(&a)) {
        eprintln!("错误：{e}");
        std::process::exit(1);
    }
}

/// Retries network trouble a few times; servers rate-limit bursts.
fn retry<T>(what: &str, f: impl Fn() -> Result<T, FetchError>) -> Result<T, String> {
    let mut last = String::new();
    for attempt in 0..4 {
        match f() {
            Ok(v) => return Ok(v),
            Err(FetchError::Unusable(e)) => return Err(format!("{what}：{e}")),
            Err(FetchError::Transient(e)) => {
                last = e;
                std::thread::sleep(Duration::from_secs(5 << attempt));
            }
        }
    }
    Err(format!("{what}：{last}"))
}

fn run(args: &Args) -> Result<(), String> {
    let review_path = args.assets.join("catalog-review.json");
    let review: Review = match fs::read(&review_path) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("{}：{e}", review_path.display()))?,
        Err(_) => Review::default(),
    };
    fs::create_dir_all(args.work.join("thumbs")).map_err(|e| e.to_string())?;
    fs::create_dir_all(args.work.join("review")).map_err(|e| e.to_string())?;
    let client = Client::new();
    let mut report: Vec<(String, String)> = Vec::new();

    let mut cands = candidates(&client)?;
    println!("候选：{}", tally(cands.values()));

    let files: Vec<String> = cands.values().map(|c| c.file.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let info = retry("Commons 图片信息", || wiki::image_info(&client, &files))?;
    let mut shapes: HashMap<String, Shape> = HashMap::new();
    cands.retain(|qid, c| {
        let verdict = match info.get(&c.file) {
            None => Err("Commons 上找不到图".to_string()),
            Some(i) if !matches!(i.mime.as_str(), "image/jpeg" | "image/png" | "image/tiff") => Err(format!("图片格式 {}", i.mime)),
            Some(i) if !i.public_domain() => Err(format!("图片授权是 {}", i.license)),
            Some(i) if i.height == 0 => Err("图片尺寸不明".to_string()),
            Some(i) => art::shape(i.width as f32 / i.height as f32),
        };
        match verdict {
            Ok(shape) => {
                shapes.insert(qid.clone(), shape);
                true
            }
            Err(why) => {
                report.push((qid.clone(), why));
                false
            }
        }
    });
    println!("图片可用：{}", cands.len());

    let ids: Vec<String> = cands.keys().cloned().collect();
    let ents = retry("Wikidata 作品", || wiki::entities(&client, &ids, "labels|descriptions|claims", LANGS))?;
    let refs = references(&client, &cands, &ents)?;

    let mut passed: Vec<Candidate> = Vec::new();
    for c in cands.values() {
        let Some(e) = ents.get(&c.qid) else {
            report.push((c.qid.clone(), "Wikidata 里没有".into()));
            continue;
        };
        match screen(c, e, shapes[&c.qid], &refs, &review) {
            Ok(()) => passed.push(c.clone()),
            Err(why) => report.push((c.qid.clone(), why)),
        }
    }
    println!("内容筛选后：{}", passed.len());

    let fits = score(&client, &args.work, &passed, &info, &review);
    let mut accepted: Vec<(Candidate, Fit, Signature)> = Vec::new();
    for c in passed {
        match &fits[&c.qid] {
            Ok((fit, sig)) => accepted.push((c, *fit, *sig)),
            Err(why) => report.push((c.qid.clone(), why.clone())),
        }
    }
    let chosen = dedupe(accepted, &mut report);
    println!("入选：{}", tally(chosen.iter().map(|(c, _)| c)));

    let titles: Vec<String> = chosen
        .iter()
        .flat_map(|(c, _)| intro_sources(c, &ents[&c.qid], &refs).into_iter().map(|(t, _)| t))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let articles = retry("中文维基百科", || wiki::zh_articles(&client, &titles))?;
    let mut entries: Vec<(u8, Entry)> = Vec::new();
    for (c, fit) in &chosen {
        let e = entry(c, &ents[&c.qid], fit, &info[&c.file], &refs, &articles, &review);
        if e.meta.intro.is_some() {
            entries.push((c.group.rank(), e));
        } else {
            report.push((c.qid.clone(), "作品、系列和作者都没有中文维基条目，没有讲解".into()));
        }
    }
    entries.sort_by(|a, b| (a.0, &a.1.meta.id).cmp(&(b.0, &b.1.meta.id)));
    let entries: Vec<Entry> = entries.into_iter().map(|(_, e)| e).collect();

    let out = args.assets.join("catalog.json");
    let json = serde_json::to_vec_pretty(&entries).map_err(|e| e.to_string())?;
    fs::write(&out, json).map_err(|e| format!("{}：{e}", out.display()))?;
    write_report(&args.work, &report, &entries)?;
    println!(
        "写入 {}：{} 幅{}",
        out.display(),
        entries.len(),
        if entries.len() < TARGET { format!("（少于 {TARGET} 幅，需要扩大来源）") } else { String::new() }
    );
    Ok(())
}

fn tally<'a>(cands: impl Iterator<Item = &'a Candidate>) -> String {
    let mut n = [0; 4];
    for c in cands {
        n[c.group.rank() as usize] += 1;
    }
    GROUP_NAMES.iter().zip(n).map(|(g, k)| format!("{g} {k}")).collect::<Vec<_>>().join("，")
}

fn candidates(client: &Client) -> Result<BTreeMap<String, Candidate>, String> {
    let famous = format!(
        "{PREFIXES}
SELECT ?item ?img ?sl ?article WHERE {{
  VALUES ?cls {{ {FAMOUS_CLASSES} }}
  ?item wdt:P31 ?cls ; wdt:P18 ?img ; wikibase:sitelinks ?sl .
  ?page schema:about ?item ; schema:isPartOf <https://zh.wikipedia.org/> ; schema:name ?article .
}}"
    );
    let series_ids = SERIES.iter().map(|(q, _)| format!("wd:{q}")).collect::<Vec<_>>().join(" ");
    let series = format!(
        "{PREFIXES}
SELECT ?item ?img ?sl ?series WHERE {{
  VALUES ?series {{ {series_ids} }}
  ?item wdt:P179|wdt:P361 ?series ; wdt:P18 ?img ; wikibase:sitelinks ?sl .
}}"
    );
    // The creator's death date is checked again later; here it narrows the
    // search enough for the label filter to finish in time.
    let known = format!(
        "{PREFIXES}
SELECT DISTINCT ?item ?img ?sl WHERE {{
  VALUES ?cls {{ {FAMOUS_CLASSES} }}
  ?item wdt:P31 ?cls ; wdt:P18 ?img ; wikibase:sitelinks ?sl ; wdt:P170 ?creator ; rdfs:label ?label .
  ?creator wdt:P570 ?died .
  FILTER(YEAR(?died) <= {DIED_BY})
  FILTER(?sl >= {KNOWN_MIN_SITELINKS})
  FILTER(LANG(?label) IN ({NAME_LANGS}))
}}"
    );
    let prints = format!(
        "{PREFIXES}
SELECT ?item ?img ?sl ?creator WHERE {{
  VALUES ?cls {{ {PRINT_CLASSES} }}
  VALUES ?creator {{ {PRINTMAKERS} }}
  ?item wdt:P31 ?cls ; wdt:P170 ?creator ; wdt:P18 ?img ; wikibase:sitelinks ?sl .
  FILTER(?sl >= {PRINT_MIN_SITELINKS})
}}"
    );
    // Earlier groups win when a work turns up in several.
    let mut out: BTreeMap<String, Candidate> = BTreeMap::new();
    for (query, kind) in [(famous, 0), (series, 1), (known, 2), (prints, 3)] {
        for row in retry("Wikidata 查询", || wiki::sparql(client, &query))? {
            let (Some(qid), Some(img)) = (row.get("item"), row.get("img")) else {
                continue;
            };
            let group = match kind {
                0 => Group::Famous,
                1 => Group::Series(row.get("series").cloned().unwrap_or_default()),
                2 => Group::Known,
                _ => Group::Prints(row.get("creator").cloned().unwrap_or_default()),
            };
            out.entry(qid.clone()).or_insert_with(|| Candidate {
                qid: qid.clone(),
                group,
                file: wiki::file_name(img),
                fame: row.get("sl").and_then(|s| s.parse().ok()).unwrap_or(0),
                article: row.get("article").cloned(),
            });
        }
    }
    Ok(out)
}

/// Fetches what the works point to. Creators and series come in full, for their
/// dates and Wikipedia articles; the rest only need names.
fn references(client: &Client, cands: &BTreeMap<String, Candidate>, ents: &HashMap<String, Value>) -> Result<Refs, String> {
    let mut full: BTreeSet<String> = BTreeSet::new();
    let mut named: BTreeSet<String> = BTreeSet::new();
    for c in cands.values() {
        if let Group::Series(s) = &c.group {
            full.insert(s.clone());
        }
        let Some(e) = ents.get(&c.qid) else { continue };
        full.extend(wiki::claim_ids(e, "P170"));
        for (p, max) in [("P195", 3), ("P136", 4), ("P180", 12), ("P135", 3), ("P186", 6)] {
            named.extend(wiki::claim_ids(e, p).into_iter().take(max));
        }
    }
    let named: Vec<String> = named.difference(&full).cloned().collect();
    let full: Vec<String> = full.into_iter().collect();
    Ok(Refs {
        full: retry("Wikidata 作者与系列", || wiki::entities(client, &full, "labels|descriptions|claims|sitelinks", LANGS))?,
        labels: retry("Wikidata 名称", || wiki::entities(client, &named, "labels", "zh-cn|zh-hans|zh|en"))?,
    })
}

fn creators(e: &Value) -> Vec<String> {
    wiki::claim_ids(e, "P170").into_iter().filter(|q| q != ANONYMOUS_QID).collect()
}

/// The checks that need only the work's data, cheapest first.
fn screen(c: &Candidate, e: &Value, shape: Shape, refs: &Refs, review: &Review) -> Result<(), String> {
    if let Some(why) = review.exclude.get(&c.qid) {
        return Err(format!("评审剔除：{why}"));
    }
    let makers = creators(e);
    if makers.is_empty() {
        match wiki::year_of(e, "P571") {
            Some(y) if y <= MADE_BY => {}
            _ => return Err("作者和年代不明，不能确认是公有领域".into()),
        }
    }
    for m in &makers {
        match refs.full.get(m).and_then(|p| wiki::year_of(p, "P570")) {
            Some(y) if y <= DIED_BY => {}
            Some(y) => return Err(format!("作者卒于 {y} 年，还在版权期内")),
            None => return Err("作者卒年不明，不能确认是公有领域".into()),
        }
    }
    let mut texts: Vec<String> = wiki::ZH_LANGS
        .iter()
        .chain(&["en", "ja"])
        .filter_map(|l| wiki::lang_text(&e["labels"], l))
        .collect();
    texts.extend(wiki::lang_text(&e["descriptions"], "en"));
    let genres = wiki::claim_ids(e, "P136");
    for id in genres.iter().chain(&wiki::claim_ids(e, "P180")) {
        texts.extend(refs.en(id));
        texts.extend(refs.zh(id));
    }
    if let Some(word) = wiki::nudity(texts.iter().map(String::as_str)) {
        return Err(format!("裸体题材（{word}）"));
    }
    if wiki::claim_ids(e, "P186").iter().filter_map(|m| refs.en(m)).any(|m| m.to_lowercase().contains("silk")) {
        return Err("绢本，底色发黄，上屏发灰".into());
    }
    if shape == Shape::Upright && !matches!(c.group, Group::Series(_)) {
        let title = wiki::lang_text(&e["labels"], "en").unwrap_or_default();
        let kinds: Vec<String> = genres.iter().flat_map(|g| refs.en(g).into_iter().chain(refs.zh(g))).collect();
        if !wiki::scenery(&title, &kinds) {
            return Err("竖幅且不是风景，裁成横幅会切掉主体".into());
        }
    }
    if chinese_title(c, e, refs, review, None).is_none() {
        return Err("没有中文标题".into());
    }
    Ok(())
}

/// Downloads a small copy of each picture and runs the panel checks on it, keeping
/// the band as a PNG for review.
fn score(
    client: &Client,
    work: &Path,
    cands: &[Candidate],
    info: &HashMap<String, ImageInfo>,
    review: &Review,
) -> HashMap<String, Result<(Fit, Signature), String>> {
    let results = Mutex::new(HashMap::new());
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..SCORE_THREADS {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(c) = cands.get(i) else { break };
                let r = score_one(client, work, c, &info[&c.file], review.focus.get(&c.qid).copied());
                results.lock().expect("no thread panics while holding the lock").insert(c.qid.clone(), r);
                if (i + 1).is_multiple_of(100) {
                    println!("打分 {}/{}", i + 1, cands.len());
                }
            });
        }
    });
    results.into_inner().expect("no thread panicked")
}

fn score_one(
    client: &Client,
    work: &Path,
    c: &Candidate,
    info: &ImageInfo,
    focus: Option<[f32; 2]>,
) -> Result<(Fit, Signature), String> {
    let aspect = info.width as f32 / info.height as f32;
    let width = SCORE_WIDTHS
        .into_iter()
        .find(|&w| w as f32 / aspect >= MIN_SIDE)
        .unwrap_or(wiki::IMAGE_WIDTH);
    let thumb = work.join("thumbs").join(format!("{}-{width}.img", c.qid));
    let bytes = match fs::read(&thumb) {
        Ok(b) => b,
        Err(_) => {
            let b = retry("下载小图", || client.get(&info.thumb(width), &[], 32 << 20))?;
            fs::write(&thumb, &b).map_err(|e| e.to_string())?;
            b
        }
    };
    let img = image::load_from_memory(&bytes).map_err(|e| format!("图片无法解码：{e}"))?.to_luma8();
    let fit = art::check_quality(&img, focus, W, ART_H)?;
    art::check_midtones(&fit)?;
    let toned = art::tone(img);
    let band = art::band(&toned, W, ART_H, fit.focus, false);
    let _ = band.save(work.join("review").join(format!("{}.png", c.qid)));
    Ok((fit, signature(&toned)))
}

fn signature(img: &GrayImage) -> Signature {
    let small = imageops::resize(img, 16, 16, imageops::FilterType::Triangle);
    small.into_raw().try_into().expect("16×16 grey is 256 bytes")
}

fn difference(a: &Signature, b: &Signature) -> f32 {
    a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y) as f32).sum::<f32>() / a.len() as f32
}

/// Keeps one entry per picture, the one in the better group or the better known.
fn dedupe(mut accepted: Vec<(Candidate, Fit, Signature)>, report: &mut Vec<(String, String)>) -> Vec<(Candidate, Fit)> {
    accepted.sort_by_key(|(c, _, _)| (c.group.rank(), Reverse(c.fame), c.qid.clone()));
    let mut kept: Vec<(Candidate, Fit, Signature)> = Vec::new();
    for (c, fit, sig) in accepted {
        let twin = kept
            .iter()
            .map(|(k, _, s)| (k, difference(s, &sig)))
            .filter(|&(_, d)| d < DUPLICATE_DIFF)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match twin {
            Some((k, d)) => report.push((c.qid.clone(), format!("和 {} 是同一幅图（差 {d:.1}）", k.qid))),
            None => kept.push((c, fit, sig)),
        }
    }
    kept.into_iter().map(|(c, fit, _)| (c, fit)).collect()
}

/// The Chinese Wikipedia articles the guide may quote, best first, each with the
/// heading it goes under: the work's own, its series', its creator's.
fn intro_sources(c: &Candidate, e: &Value, refs: &Refs) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    match &c.group {
        Group::Famous => out.extend(c.article.clone().map(|t| (t, None))),
        Group::Series(s) => out.extend(refs.zhwiki(s).map(|t| (t, Some(format!("关于《{}》", series_name(s)))))),
        Group::Known | Group::Prints(_) => {}
    }
    if let Some(t) = creators(e).first().and_then(|m| refs.zhwiki(m)) {
        out.push((t, Some("关于画家".into())));
    }
    out
}

/// Drops a trailing disambiguation such as " (莫奈)" or "（浮世绘）", and a quote
/// mark left open.
fn plain_title(s: &str) -> String {
    let s = s.trim();
    let s = match (s.strip_prefix('“'), s.contains('”')) {
        (Some(rest), false) => rest,
        _ => s,
    };
    for (open, close) in [(" (", ")"), ("（", "）"), ("(", ")")] {
        if s.ends_with(close) {
            if let Some(at) = s.rfind(open) {
                if at > 0 {
                    return s[..at].trim().to_string();
                }
            }
        }
    }
    s.to_string()
}

/// Text a Chinese reader can take as a title: Japanese の reads as 之, as in
/// the usual Chinese renderings of print titles; any other kana disqualifies it.
fn readable(s: &str) -> Option<String> {
    let s = s.replace(['の', 'ノ'], "之");
    (!wiki::has_kana(&s)).then(|| zh::from_japanese(&s))
}

/// Drops the series name that print titles often start with, which the guide
/// already shows: "东海道五拾三次之内 府中 安部川" → "府中 安部川".
fn strip_series(title: &str) -> String {
    let mut prefixes: Vec<&str> = SERIES.iter().map(|(_, name)| *name).chain(TITLE_PREFIXES.iter().copied()).collect();
    prefixes.sort_by_key(|p| Reverse(p.len()));
    let mut t = title.trim();
    for p in prefixes {
        if let Some(rest) = t.strip_prefix(p) {
            let rest = rest.strip_prefix("之内").unwrap_or(rest);
            t = rest.trim_start_matches([' ', '\u{3000}', '・', '·', '：', ':']);
            break;
        }
    }
    t.to_string()
}

/// A creator's name in kanji, as Japanese gives it for East Asian artists; Western
/// names are always in kana.
fn kanji_name(p: &Value) -> Option<String> {
    wiki::lang_text(&p["labels"], "ja").and_then(|n| readable(&n))
}

/// The title shown: a reviewer's, else the work's Chinese article's, the Chinese
/// label, or the Japanese title of an East Asian work. Japanese titles of Western
/// works are translations that read oddly in Chinese ("草上之昼食").
fn chinese_title(c: &Candidate, e: &Value, refs: &Refs, review: &Review, own: Option<&Article>) -> Option<String> {
    if let Some(t) = review.title.get(&c.qid) {
        return Some(t.clone());
    }
    let labels = &e["labels"];
    let east_asian = creators(e).iter().all(|m| refs.full.get(m).and_then(kanji_name).is_some());
    let raw = own
        .map(|a| a.title.clone())
        .or_else(|| wiki::zh_text(labels).and_then(|t| readable(&t)))
        .or_else(|| c.article.as_deref().and_then(readable))
        .or_else(|| wiki::lang_text(labels, "ja").filter(|_| east_asian).and_then(|j| readable(&j)))?;
    let t = strip_series(&plain_title(&raw));
    (!t.is_empty()).then_some(t)
}

fn entry(
    c: &Candidate,
    e: &Value,
    fit: &Fit,
    info: &ImageInfo,
    refs: &Refs,
    articles: &HashMap<String, Article>,
    review: &Review,
) -> Entry {
    let own = c.article.as_ref().and_then(|t| articles.get(t));
    let title = chinese_title(c, e, refs, review, own).unwrap_or_else(|| c.qid.clone());
    let makers = creators(e);
    let maker = makers.first();
    let artist = maker
        .and_then(|m| {
            refs.zh(m)
                .map(|n| zh::name(&n))
                .or_else(|| refs.full.get(m).and_then(kanji_name))
                .or_else(|| refs.en(m))
        })
        .unwrap_or_else(|| ANONYMOUS.into());
    let artist_original = maker.and_then(|m| {
        let name = refs.en(m)?;
        let p = refs.full.get(m)?;
        Some(match (wiki::year_of(p, "P569"), wiki::year_of(p, "P570")) {
            (Some(b), Some(d)) => format!("{name} ({b}–{d})"),
            _ => name,
        })
    });
    let genre = refs.zh_list(&wiki::claim_ids(e, "P136"), 2);
    let movement = refs.zh_list(&wiki::claim_ids(e, "P135"), 2);
    let subjects: Vec<String> = refs
        .zh_list(&wiki::claim_ids(e, "P180"), 8)
        .into_iter()
        .filter(|s| !genre.contains(s))
        .take(6)
        .collect();
    let materials: Vec<(String, Option<String>)> = wiki::claim_ids(e, "P186")
        .iter()
        .map(|m| (refs.en(m).unwrap_or_default(), refs.zh(m)))
        .collect();
    let quoted = intro_sources(c, e, refs).into_iter().find_map(|(t, heading)| {
        let a = articles.get(&t)?;
        let text = wiki::trim_intro(&a.intro, INTRO_MAX);
        (!text.is_empty()).then_some((a, text, heading))
    });
    let round = |x: f32| (x * 100.0).round() / 100.0;
    let meta = ArtMeta {
        id: c.qid.clone(),
        title,
        artist,
        year: wiki::made(e),
        title_original: wiki::lang_text(&e["labels"], "en"),
        artist_original,
        artist_about: maker.and_then(|m| refs.full.get(m)).and_then(|p| wiki::zh_text(&p["descriptions"])),
        genre: (!genre.is_empty()).then(|| genre.join("、")),
        movement: (!movement.is_empty()).then(|| movement.join("、")),
        medium: wiki::medium(&materials),
        dimensions: wiki::dimensions(e),
        subjects,
        museum: wiki::claim_ids(e, "P195").iter().find_map(|m| refs.zh(m)),
        intro_source: quoted.as_ref().map(|(a, _, _)| format!("摘自维基百科「{}」条目，CC BY-SA 4.0", a.title)),
        intro_heading: quoted.as_ref().and_then(|(_, _, h)| h.clone()),
        intro: quoted.map(|(_, text, _)| text),
        focus: fit.focus.map(|f| [round(f[0]), round(f[1])]),
        source: format!("Wikimedia Commons，公有领域；Wikidata {}", c.qid),
        url: own.map(|a| a.url.clone()).or_else(|| Some(info.page.clone()).filter(|p| !p.is_empty())),
        ..ArtMeta::default()
    };
    Entry {
        meta,
        image: info.url.clone(),
        fame: c.fame,
        group: match &c.group {
            Group::Famous | Group::Known => FAMOUS.into(),
            Group::Series(s) => series_name(s).into(),
            Group::Prints(_) => "版画".into(),
        },
        midtones: (fit.midtones * 1000.0).round() / 1000.0,
    }
}

fn write_report(work: &Path, report: &[(String, String)], entries: &[Entry]) -> Result<(), String> {
    let mut out = String::from("qid\t结果\n");
    for e in entries {
        out.push_str(&format!("{}\t入选 {} 中间灰 {:.2} 条目数 {}《{}》\n", e.meta.id, e.group, e.midtones, e.fame, e.meta.title));
    }
    for (qid, why) in report {
        out.push_str(&format!("{qid}\t{why}\n"));
    }
    let path = work.join("report.tsv");
    fs::write(&path, out).map_err(|e| format!("{}：{e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_titles_lose_the_series_name() {
        assert_eq!(strip_series("东海道五拾三次之内 府中 安部川"), "府中 安部川");
        assert_eq!(strip_series("富岳三十六景　甲州石班泽"), "甲州石班泽");
        assert_eq!(strip_series("东海道五十三次"), "");
        assert_eq!(strip_series("京都名所之内 金阁寺"), "金阁寺");
        assert_eq!(strip_series("神奈川冲浪里"), "神奈川冲浪里");
    }

    #[test]
    fn kana_titles_are_read_or_refused() {
        assert_eq!(readable("隅田川関屋の里").as_deref(), Some("隅田川关屋之里"));
        assert_eq!(readable("東海道五十三次 程ヶ谷"), None);
        assert_eq!(readable("燕子花圖"), Some("燕子花图".into()));
    }
}
