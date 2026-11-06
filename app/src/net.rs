//! Blocking HTTP shared by the catalogue builder and the picture download. Run it
//! off the async runtime.
use serde_json::Value;
use std::{fmt, io::Read, time::Duration};

const USER_AGENT: &str = concat!(
    "ArtClock/",
    env!("CARGO_PKG_VERSION"),
    " (ZECTRIX NOTE4 e-ink clock; https://www.zectrix.com)"
);
const MAX_JSON: u64 = 32 << 20;

#[derive(Debug)]
pub enum FetchError {
    /// The object cannot be shown (missing, not a picture, unreadable on the panel).
    Unusable(String),
    /// Network or server trouble; worth trying again later.
    Transient(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Unusable(why) | FetchError::Transient(why) => f.write_str(why),
        }
    }
}

pub struct Client {
    agent: ureq::Agent,
}

impl Default for Client {
    fn default() -> Self {
        Self::new()
    }
}

impl Client {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .user_agent(USER_AGENT)
            .build();
        Self { agent }
    }

    /// GET with query parameters, reading at most `limit` bytes of the body.
    pub fn get(&self, url: &str, params: &[(&str, &str)], limit: u64) -> Result<Vec<u8>, FetchError> {
        let mut req = self.agent.get(url).set("Accept", "application/sparql-results+json, application/json, */*");
        for (k, v) in params {
            req = req.query(k, v);
        }
        let resp = req.call().map_err(|e| match e {
            ureq::Error::Status(code @ (404 | 410), _) => FetchError::Unusable(format!("{url} 返回 {code}")),
            ureq::Error::Status(code, _) => FetchError::Transient(format!("{url} 返回 {code}")),
            ureq::Error::Transport(t) => FetchError::Transient(format!("{url}：{t}")),
        })?;
        let mut body = Vec::new();
        resp.into_reader()
            .take(limit)
            .read_to_end(&mut body)
            .map_err(|e| FetchError::Transient(format!("{url}：{e}")))?;
        Ok(body)
    }

    pub fn json(&self, url: &str, params: &[(&str, &str)]) -> Result<Value, FetchError> {
        let body = self.get(url, params, MAX_JSON)?;
        serde_json::from_slice(&body).map_err(|e| FetchError::Transient(format!("{url}：{e}")))
    }
}
