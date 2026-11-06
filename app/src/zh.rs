//! Chinese text helpers: Traditional to Simplified, and tidy names.
use std::{collections::HashMap, sync::OnceLock};

/// OpenCC's Traditional→Simplified character table (Apache-2.0, see the file header).
const TS_TABLE: &str = include_str!("../data/TSCharacters.txt");

fn ts_map() -> &'static HashMap<char, char> {
    static MAP: OnceLock<HashMap<char, char>> = OnceLock::new();
    MAP.get_or_init(|| {
        TS_TABLE
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (from, to) = l.split_once('\t')?;
                let mut from = from.chars();
                let mut to = to.split(' ').next()?.chars();
                match (from.next(), from.next(), to.next(), to.next()) {
                    (Some(f), None, Some(t), None) => Some((f, t)),
                    _ => None,
                }
            })
            .collect()
    })
}

/// Character-level conversion; good enough for names and short labels.
pub fn simplified(s: &str) -> String {
    let map = ts_map();
    s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect()
}

/// Japanese forms that Chinese does not use, as found in print titles, paired with
/// the Simplified forms: 戸 → 户, 沢 → 泽, 図 → 图...
const JAPANESE: &str = "戸户沢泽円圆図图継继薬药縄绳豊丰竜龙亀龟嶌岛嶋岛冨富斎斋広广渓溪隠隐総总従从両两関关黒黑歩步駅驿桜樱塩盐稲稻実实売卖読读気气剣剑郷乡徳德拝拜晩晚仏佛恵惠栄荣楽乐険险験验釈释帰归戦战対对粋粹巣巢舎舍権权様样乗乘辺边姫姬滝泷暁晓渉涉撃击闘斗弾弹鉄铁銭钱囲围県县伝传転转庁厅砕碎雑杂巌岩絵绘霊灵収收蔵藏";

/// Traditional and Japanese forms to Simplified.
pub fn from_japanese(s: &str) -> String {
    let pairs: Vec<char> = JAPANESE.chars().collect();
    simplified(s)
        .chars()
        .map(|c| pairs.chunks(2).find(|p| p[0] == c).map_or(c, |p| p[1]))
        .collect()
}

/// Wikidata names mix several middle dots; the panel font draws `·` best.
pub fn name(s: &str) -> String {
    s.trim().replace(['・', '‧', '•', '∙'], "·")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traditional_becomes_simplified() {
        assert_eq!(simplified("喬治·貝洛斯"), "乔治·贝洛斯");
        assert_eq!(simplified("後印象派荷蘭藝術家"), "后印象派荷兰艺术家");
        assert_eq!(simplified("麦田与丝柏"), "麦田与丝柏");
    }

    #[test]
    fn japanese_forms_become_chinese() {
        assert_eq!(from_japanese("江戸日本橋"), "江户日本桥");
        assert_eq!(from_japanese("甲州伊沢暁"), "甲州伊泽晓");
    }
}
