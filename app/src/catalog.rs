//! The works the clock shows, one a day: famous public-domain pictures chosen ahead
//! of time by the `catalog` tool from Wikidata, with images on Wikimedia Commons.
use crate::art::ArtMeta;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// `id` is the Wikidata item, e.g. "Q252485".
    #[serde(flatten)]
    pub meta: ArtMeta,
    /// The picture on Wikimedia Commons, at most 1280 px wide.
    pub image: String,
    /// Wikipedia language editions with an article on the work.
    #[serde(default)]
    pub fame: u32,
    /// Why the work is in the list: "名作", a series such as "富岳三十六景", or "版画".
    #[serde(default)]
    pub group: String,
    /// Share of mid-grey pixels in the band when the list was built; lower is crisper.
    #[serde(default)]
    pub midtones: f32,
}

pub fn load(path: &Path) -> Result<Vec<Entry>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}：{e}", path.display()))?;
    let list: Vec<Entry> = serde_json::from_slice(&bytes).map_err(|e| format!("{}：{e}", path.display()))?;
    if list.is_empty() {
        return Err(format!("{} 里没有作品", path.display()));
    }
    Ok(list)
}
