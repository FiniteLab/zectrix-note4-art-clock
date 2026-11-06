//! Building the catalogue on a development machine: famous public-domain works from
//! Wikidata, their images on Wikimedia Commons, and Chinese introductions from
//! Chinese Wikipedia, which the clock itself often cannot reach. Used by the
//! `catalog` tool; the clock only reads its output.
use crate::{
    net::{Client, FetchError},
    zh,
};
use serde_json::Value;
use std::collections::HashMap;

/// QLever answers whole-of-Wikidata queries in seconds; query.wikidata.org times out on them.
pub const QLEVER: &str = "https://qlever.dev/api/wikidata";
const WIKIDATA: &str = "https://www.wikidata.org/w/api.php";
const COMMONS: &str = "https://commons.wikimedia.org/w/api.php";
const ZHWIKI: &str = "https://zh.wikipedia.org/w/api.php";
/// Mainland usage first. The others are often Traditional, and even mainland
/// labels sometimes mix in Traditional characters, so every label is converted.
pub const ZH_LANGS: [&str; 6] = ["zh-cn", "zh-hans", "zh", "zh-hant", "zh-tw", "zh-hk"];
/// The clock downloads this width, one of those Commons keeps ready.
pub const IMAGE_WIDTH: u32 = 1280;

/// Subjects that usually mean nudity. The clock sits in shared rooms, so such works
/// are skipped; matching is on whole words and errs on the side of skipping.
const NUDITY_WORDS: &[&str] = &[
    "nude", "nudes", "nudity", "naked", "undressed", "bath", "bather", "bathers", "bathing",
    "odalisque", "venus", "leda", "danae", "danaë", "susanna", "bathsheba", "andromeda",
    "nymph", "nymphs", "satyr", "satyrs", "bacchanal", "bacchanalia", "toilette", "eros", "cupid",
];
const NUDITY_PHRASES: &[&str] = &[
    "adam and eve",
    "three graces",
    "judgment of paris",
    "judgement of paris",
    "diana and actaeon",
    "saint sebastian",
    "st. sebastian",
];
/// Title words that mark scenery. Upright works lose most of their height when
/// cropped to the band, which scenery survives; other upright works are skipped.
const SCENERY_WORDS: &[&str] = &[
    "landscape", "landscapes", "seascape", "cityscape", "view", "vista", "river", "lake", "pond",
    "sea", "coast", "shore", "beach", "harbor", "harbour", "bay", "cliffs", "rocks", "mountain",
    "mountains", "hills", "valley", "gorge", "waterfall", "falls", "forest", "woods", "trees",
    "grove", "meadow", "field", "fields", "marsh", "garden", "park", "bridge", "canal", "road",
    "village", "moon", "moonrise", "moonlight", "sunrise", "sunset", "twilight", "snow", "storm",
    "clouds",
];
/// Genre and tag fragments that mark scenery, in English or Chinese.
const SCENERY_KINDS: &[&str] = &["landscape", "seascape", "cityscape", "marine", "veduta", "风景", "山水", "海景"];
/// Title words that mark a person as the subject, which a wide crop would cut apart.
const FIGURE_WORDS: &[&str] = &[
    "portrait", "self", "madonna", "virgin", "christ", "saint", "woman", "women", "man", "men",
    "girl", "boy", "lady", "child", "children", "mother", "family", "mrs", "mr", "miss", "madame",
    "mme", "head", "bust", "figure", "figures",
];
const FIGURE_KINDS: &[&str] = &["portrait", "figure", "肖像", "人像"];

/// Lowercase words; dots are kept inside a word ("st.") and trimmed from its ends.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '.'))
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// The nudity keyword found in `texts`, if any.
pub fn nudity<'a>(texts: impl IntoIterator<Item = &'a str>) -> Option<&'static str> {
    for text in texts {
        if text.contains('裸') {
            return Some("裸");
        }
        let words = words(text);
        if let Some(k) = NUDITY_WORDS.iter().find(|k| words.iter().any(|w| w == *k)) {
            return Some(k);
        }
        let joined = format!(" {} ", words.join(" "));
        if let Some(p) = NUDITY_PHRASES
            .iter()
            .find(|p| joined.contains(&format!(" {} ", p.replace("st. ", "st "))))
        {
            return Some(p);
        }
    }
    None
}

/// Whether a work shows scenery, judged from its English title and its genres.
pub fn scenery(title: &str, kinds: &[String]) -> bool {
    let title = words(title);
    let in_title = |list: &[&str]| title.iter().any(|w| list.contains(&w.as_str()));
    let in_kinds = |list: &[&str]| {
        kinds.iter().any(|k| {
            let k = k.to_lowercase();
            list.iter().any(|f| k.contains(f))
        })
    };
    !in_title(FIGURE_WORDS) && !in_kinds(FIGURE_KINDS) && (in_title(SCENERY_WORDS) || in_kinds(SCENERY_KINDS))
}

/// Rows of a SPARQL SELECT, each value as text; entity IRIs become bare IDs.
pub fn sparql(client: &Client, query: &str) -> Result<Vec<HashMap<String, String>>, FetchError> {
    let v = client.json(QLEVER, &[("query", query)])?;
    let rows = v["results"]["bindings"].as_array().ok_or_else(|| {
        FetchError::Transient(format!("SPARQL 出错：{}", v["exception"].as_str().unwrap_or("没有结果")))
    })?;
    Ok(rows
        .iter()
        .map(|r| {
            r.as_object()
                .into_iter()
                .flatten()
                .map(|(k, b)| {
                    let s = b["value"].as_str().unwrap_or("");
                    (k.clone(), s.strip_prefix("http://www.wikidata.org/entity/").unwrap_or(s).to_string())
                })
                .collect()
        })
        .collect())
}

/// Wikidata entities by ID, fetched 50 at a time.
pub fn entities(client: &Client, ids: &[String], props: &str, langs: &str) -> Result<HashMap<String, Value>, FetchError> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(50) {
        let joined = chunk.join("|");
        let params = [
            ("action", "wbgetentities"),
            ("format", "json"),
            ("ids", joined.as_str()),
            ("props", props),
            ("languages", langs),
        ];
        let v = client.json(WIKIDATA, &params)?;
        for (id, e) in v["entities"].as_object().into_iter().flatten() {
            if e.get("missing").is_none() {
                out.insert(id.clone(), e.clone());
            }
        }
    }
    Ok(out)
}

fn statements<'a>(e: &'a Value, property: &str) -> impl Iterator<Item = &'a Value> {
    e["claims"][property].as_array().into_iter().flatten()
}

/// Item values of a property, e.g. the creators of a work.
pub fn claim_ids(e: &Value, property: &str) -> Vec<String> {
    statements(e, property)
        .filter_map(|c| c["mainsnak"]["datavalue"]["value"]["id"].as_str())
        .map(str::to_string)
        .collect()
}

/// String values of a property, e.g. Commons file names.
pub fn claim_strings(e: &Value, property: &str) -> Vec<String> {
    statements(e, property)
        .filter_map(|c| c["mainsnak"]["datavalue"]["value"].as_str())
        .map(str::to_string)
        .collect()
}

/// The first Simplified Chinese value of a Wikidata labels or descriptions object.
pub fn zh_text(values: &Value) -> Option<String> {
    ZH_LANGS.iter().find_map(|&lang| {
        let s = values[lang]["value"].as_str()?.trim();
        (!s.is_empty()).then(|| zh::simplified(s))
    })
}

pub fn lang_text(values: &Value, lang: &str) -> Option<String> {
    values[lang]["value"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn time_year(v: &Value) -> Option<i64> {
    let t = v["time"].as_str()?;
    let (sign, rest) = t.split_at(1);
    let y: i64 = rest.split('-').next()?.parse().ok()?;
    Some(if sign == "-" { -y } else { y })
}

/// The year a property's first value falls in, e.g. a creator's death (P570).
pub fn year_of(e: &Value, property: &str) -> Option<i64> {
    statements(e, property).find_map(|c| time_year(&c["mainsnak"]["datavalue"]["value"]))
}

/// When a work was made, in Chinese: "1831", "约 1665", "1830–1832", "1880 年代", "16 世纪".
pub fn made(e: &Value) -> String {
    let Some(c) = statements(e, "P571").next() else {
        return String::new();
    };
    let v = &c["mainsnak"]["datavalue"]["value"];
    let Some(y) = time_year(v) else {
        return String::new();
    };
    let q = &c["qualifiers"];
    let qualifier_year = |p: &str| q[p].as_array().and_then(|a| a.first()).and_then(|s| time_year(&s["datavalue"]["value"]));
    let circa = q["P1480"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|s| s["datavalue"]["value"]["id"] == "Q5727902");
    let body = match v["precision"].as_u64().unwrap_or(9) {
        p if p >= 9 => match (qualifier_year("P1319"), qualifier_year("P1326")) {
            (Some(a), Some(b)) if a < b => format!("{a}–{b}"),
            _ => y.to_string(),
        },
        8 => format!("{} 年代", y - y.rem_euclid(10)),
        7 => format!("{} 世纪", (y - 1).div_euclid(100) + 1),
        _ => return String::new(),
    };
    if circa {
        format!("约 {body}")
    } else {
        body
    }
}

/// Height × width in centimetres, from P2048 and P2049.
pub fn dimensions(e: &Value) -> Option<String> {
    let cm = |p: &str| -> Option<f64> {
        let v = &statements(e, p).next()?["mainsnak"]["datavalue"]["value"];
        let amount: f64 = v["amount"].as_str()?.trim_start_matches('+').parse().ok()?;
        match v["unit"].as_str()?.rsplit('/').next()? {
            "Q174728" => Some(amount),
            "Q174789" => Some(amount / 10.0),
            "Q11573" => Some(amount * 100.0),
            "Q218593" => Some(amount * 2.54),
            _ => None,
        }
    };
    let show = |x: f64| {
        let r = (x * 10.0).round() / 10.0;
        if r.fract() == 0.0 {
            format!("{r:.0}")
        } else {
            format!("{r:.1}")
        }
    };
    Some(format!("{} × {} 厘米", show(cm("P2048")?), show(cm("P2049")?)))
}

/// The usual Chinese name for common techniques and paint-on-support pairs, else
/// the materials' Chinese names. Each material is (English, Chinese) as Wikidata
/// labels them; English is only used for matching, so the panel never mixes scripts.
pub fn medium(materials: &[(String, Option<String>)]) -> Option<String> {
    let has = |w: &str| materials.iter().any(|(en, _)| en.to_lowercase().contains(w));
    let named = [
        (has("woodblock"), "木版画"),
        (has("woodcut"), "木刻版画"),
        (has("mezzotint"), "美柔汀版画"),
        (has("aquatint"), "飞尘蚀刻版画"),
        (has("etching"), "蚀刻版画"),
        (has("drypoint"), "干刻版画"),
        (has("engraving"), "雕版画"),
        (has("lithograph"), "石版画"),
        (has("fresco"), "湿壁画"),
        (has("oil") && has("canvas"), "布面油画"),
        (has("oil") && (has("panel") || has("wood") || has("oak") || has("poplar")), "木板油画"),
        (has("oil") && has("copper"), "铜板油画"),
        (has("oil") && (has("paper") || has("cardboard")), "纸面油画"),
        (has("tempera") && (has("panel") || has("wood")), "木板蛋彩画"),
        (has("watercolo") && has("paper"), "纸本水彩"),
        (has("ink") && has("paper"), "纸本水墨"),
    ];
    if let Some((_, name)) = named.iter().find(|(hit, _)| *hit) {
        return Some(name.to_string());
    }
    let mut names: Vec<String> = Vec::new();
    for zh in materials.iter().filter_map(|(_, zh)| zh.clone()) {
        if !names.contains(&zh) && names.len() < 3 {
            names.push(zh);
        }
    }
    (!names.is_empty()).then(|| names.join("、"))
}

#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub mime: String,
    /// At most `IMAGE_WIDTH` wide.
    pub url: String,
    pub license: String,
    pub page: String,
}

impl ImageInfo {
    pub fn public_domain(&self) -> bool {
        let l = self.license.to_lowercase();
        l.contains("public domain") || l.starts_with("pd") || l.contains("cc0")
    }

    /// A smaller copy, `width` px wide; pictures narrower than that come whole.
    pub fn thumb(&self, width: u32) -> String {
        self.url.replace(&format!("/{IMAGE_WIDTH}px-"), &format!("/{width}px-"))
    }
}

/// Sizes, licences and resized copies of Commons files, keyed by the names asked for.
pub fn image_info(client: &Client, files: &[String]) -> Result<HashMap<String, ImageInfo>, FetchError> {
    let mut out = HashMap::new();
    let width = IMAGE_WIDTH.to_string();
    for chunk in files.chunks(50) {
        let titles: Vec<String> = chunk.iter().map(|f| format!("File:{f}")).collect();
        let joined = titles.join("|");
        let params = [
            ("action", "query"),
            ("format", "json"),
            ("redirects", "1"),
            ("prop", "imageinfo"),
            ("iiprop", "size|mime|url|extmetadata"),
            ("iiextmetadatafilter", "LicenseShortName|License"),
            ("iiurlwidth", width.as_str()),
            ("titles", joined.as_str()),
        ];
        let v = client.json(COMMONS, &params)?;
        let asked = resolve_titles(&v, &titles);
        for page in v["query"]["pages"].as_object().into_iter().flatten().map(|(_, p)| p) {
            let Some(ii) = page["imageinfo"].as_array().and_then(|a| a.first()) else {
                continue;
            };
            let Some(name) = page["title"].as_str().and_then(|t| asked.get(t)) else {
                continue;
            };
            let strip = |u: &str| u.split('?').next().unwrap_or(u).to_string();
            let url = strip(ii["thumburl"].as_str().or(ii["url"].as_str()).unwrap_or(""));
            let meta = &ii["extmetadata"];
            let license = [&meta["License"]["value"], &meta["LicenseShortName"]["value"]]
                .into_iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" / ");
            out.insert(
                name.trim_start_matches("File:").to_string(),
                ImageInfo {
                    width: ii["width"].as_u64().unwrap_or(0) as u32,
                    height: ii["height"].as_u64().unwrap_or(0) as u32,
                    mime: ii["mime"].as_str().unwrap_or("").to_string(),
                    url,
                    license,
                    page: ii["descriptionurl"].as_str().unwrap_or("").to_string(),
                },
            );
        }
    }
    Ok(out)
}

/// Maps each final page title in a query response back to the title asked for,
/// through the API's normalisation and redirects.
fn resolve_titles(v: &Value, asked: &[String]) -> HashMap<String, String> {
    let hop = |key: &str| -> HashMap<String, String> {
        v["query"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| Some((n["from"].as_str()?.to_string(), n["to"].as_str()?.to_string())))
            .collect()
    };
    let (normalized, redirects) = (hop("normalized"), hop("redirects"));
    asked
        .iter()
        .map(|a| {
            let n = normalized.get(a).unwrap_or(a);
            let r = redirects.get(n).unwrap_or(n);
            (r.clone(), a.clone())
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct Article {
    /// In mainland usage, e.g. "神奈川冲浪里".
    pub title: String,
    /// The lead section as plain text, in mainland usage.
    pub intro: String,
    pub url: String,
}

/// Lead sections of Chinese Wikipedia articles, keyed by the titles asked for.
pub fn zh_articles(client: &Client, titles: &[String]) -> Result<HashMap<String, Article>, FetchError> {
    let mut out = HashMap::new();
    for chunk in titles.chunks(20) {
        let joined = chunk.join("|");
        let params = [
            ("action", "query"),
            ("format", "json"),
            ("redirects", "1"),
            ("prop", "extracts|info"),
            ("exintro", "1"),
            ("explaintext", "1"),
            ("exlimit", "20"),
            ("inprop", "url|varianttitles"),
            ("variant", "zh-cn"),
            ("titles", joined.as_str()),
        ];
        let v = client.json(ZHWIKI, &params)?;
        let asked = resolve_titles(&v, chunk);
        for page in v["query"]["pages"].as_object().into_iter().flatten().map(|(_, p)| p) {
            let (Some(title), Some(intro)) = (page["title"].as_str(), page["extract"].as_str()) else {
                continue;
            };
            let Some(name) = asked.get(title) else {
                continue;
            };
            let shown = page["varianttitles"]["zh-cn"].as_str().unwrap_or(title);
            out.insert(
                name.clone(),
                Article {
                    title: zh::simplified(shown),
                    intro: intro.trim().to_string(),
                    url: page["fullurl"].as_str().unwrap_or("").to_string(),
                },
            );
        }
    }
    Ok(out)
}

/// Whole paragraphs, then whole sentences, up to `max` characters.
pub fn trim_intro(text: &str, max: usize) -> String {
    let mut out = String::new();
    for para in text.split('\n').map(str::trim).filter(|p| !p.is_empty()) {
        let para = para.replace("（）", "").replace("()", "");
        let room = max.saturating_sub(out.chars().count());
        if para.chars().count() <= room {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&para);
            continue;
        }
        let head: String = para.chars().take(room).collect();
        if let Some(end) = head.rfind('。') {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&head[..end + '。'.len_utf8()]);
        }
        break;
    }
    out
}

/// "Special:FilePath/Tsunami%20by%20hokusai.jpg" → "Tsunami by hokusai.jpg".
pub fn file_name(url: &str) -> String {
    let raw = url.rsplit('/').next().unwrap_or(url);
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).replace('_', " ")
}

/// Whether text has Japanese kana, which a Chinese reader cannot read as a title.
pub fn has_kana(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudity_is_caught_by_whole_words() {
        assert_eq!(nudity(["Battle of the Nudes"]), Some("nudes"));
        assert_eq!(nudity(["Venus and Adonis"]), Some("venus"));
        assert_eq!(nudity(["The Temptation of Adam and Eve"]), Some("adam and eve"));
        assert_eq!(nudity(["St. Sebastian Tended by Irene"]), Some("st. sebastian"));
        assert_eq!(nudity(["Diana at the Bath"]), Some("bath"));
        assert_eq!(nudity(["女性裸体"]), Some("裸"));
        assert_eq!(nudity(["Wheat Field with Cypresses", "Landscapes", "Venusberg"]), None);
        assert_eq!(nudity(["Madam and Everyone"]), None);
    }

    #[test]
    fn only_scenery_passes_as_an_upright_work() {
        let kinds = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(scenery("Moonrise", &[]));
        assert!(scenery("The Harvesters", &kinds(&["landscape art"])));
        assert!(scenery("Untitled", &kinds(&["风景画"])));
        assert!(!scenery("Portrait of a Woman", &kinds(&["landscape art"])));
        assert!(!scenery("Mrs. Hugh Hammersley", &[]));
        assert!(!scenery("River Spirit", &kinds(&["肖像画"])));
        assert!(!scenery("Still Life with Flowers", &[]));
    }

    #[test]
    fn wikidata_values_become_chinese() {
        let e = serde_json::json!({"claims": {
            "P571": [{"mainsnak": {"datavalue": {"value": {"time": "+1665-00-00T00:00:00Z", "precision": 9}}},
                      "qualifiers": {"P1480": [{"datavalue": {"value": {"id": "Q5727902"}}}]}}],
            "P2048": [{"mainsnak": {"datavalue": {"value": {"amount": "+44.5", "unit": "http://www.wikidata.org/entity/Q174728"}}}}],
            "P2049": [{"mainsnak": {"datavalue": {"value": {"amount": "+0.39", "unit": "http://www.wikidata.org/entity/Q11573"}}}}]
        }});
        assert_eq!(made(&e), "约 1665");
        assert_eq!(dimensions(&e).as_deref(), Some("44.5 × 39 厘米"));
        let decade = serde_json::json!({"claims": {"P571": [{"mainsnak": {"datavalue": {"value": {"time": "+1880-00-00T00:00:00Z", "precision": 8}}}}]}});
        assert_eq!(made(&decade), "1880 年代");
        let century = serde_json::json!({"claims": {"P571": [{"mainsnak": {"datavalue": {"value": {"time": "+1550-00-00T00:00:00Z", "precision": 7}}}}]}});
        assert_eq!(made(&century), "16 世纪");
    }

    #[test]
    fn media_get_their_usual_names() {
        let m = |pairs: &[(&str, Option<&str>)]| {
            medium(&pairs.iter().map(|(e, z)| (e.to_string(), z.map(str::to_string))).collect::<Vec<_>>())
        };
        assert_eq!(m(&[("oil paint", Some("油彩")), ("canvas", Some("帆布"))]).as_deref(), Some("布面油画"));
        assert_eq!(m(&[("paper", Some("纸")), ("woodblock", None)]).as_deref(), Some("木版画"));
        assert_eq!(m(&[("graphite", Some("石墨")), ("paper", Some("纸")), ("chalk", None)]).as_deref(), Some("石墨、纸"));
        assert_eq!(m(&[]), None);
    }

    #[test]
    fn chinese_labels_prefer_simplified() {
        let v: Value = serde_json::json!({
            "zh": {"value": "喬治·貝洛斯"},
            "zh-hans": {"value": "乔治·伯罗斯"}
        });
        assert_eq!(zh_text(&v).as_deref(), Some("乔治·伯罗斯"));
        let only_zh: Value = serde_json::json!({"zh": {"value": "垃圾箱畫派"}});
        assert_eq!(zh_text(&only_zh).as_deref(), Some("垃圾箱画派"));
    }

    #[test]
    fn intros_end_at_a_sentence() {
        let text = "第一段很短。\n第二段比较长，有好几句话。这是第二句。这是第三句。";
        assert_eq!(trim_intro(text, 100), text);
        assert_eq!(trim_intro(text, 20), "第一段很短。\n第二段比较长，有好几句话。");
        assert_eq!(trim_intro("没有句号的长段落".repeat(10).as_str(), 10), "");
    }

    #[test]
    fn file_names_come_from_commons_urls() {
        assert_eq!(
            file_name("http://commons.wikimedia.org/wiki/Special:FilePath/Tsunami%20by%20hokusai%2019th_century.jpg"),
            "Tsunami by hokusai 19th century.jpg"
        );
        assert_eq!(file_name("Caf%C3%A9.jpg"), "Café.jpg");
    }
}
