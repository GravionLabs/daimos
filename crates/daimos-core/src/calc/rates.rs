//! Exchange rates from the European Central Bank's daily reference feed, cached on disk.
//!
//! The feed lists the rate of every currency per euro and the date of the fixing. The cache keeps
//! the last answer, so conversions work offline and the item can say how old the rates are.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::RwLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";

/// Rates older than this are fetched again (the ECB publishes once per working day).
pub const MAX_AGE_SECS: i64 = 20 * 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rates {
    /// The day of the fixing, `YYYY-MM-DD`.
    pub date: String,
    /// Units of the currency per euro, by uppercase code; EUR itself is 1.
    pub per_eur: HashMap<String, f64>,
    /// When the feed was fetched, seconds since the epoch.
    pub fetched_at: i64,
}

impl Rates {
    pub fn rate(&self, code: &str) -> Option<f64> {
        self.per_eur.get(&code.to_uppercase()).copied()
    }
}

/// Reads the ECB XML: `<Cube time='2026-10-09'>` once, then `<Cube currency='USD' rate='1.09'/>`.
pub fn parse_ecb(xml: &str, fetched_at: i64) -> Result<Rates, String> {
    let mut date = None;
    let mut per_eur = HashMap::from([("EUR".to_owned(), 1.0)]);
    for tag in xml.split('<').skip(1) {
        let tag = tag.split('>').next().unwrap_or("");
        if !tag.starts_with("Cube") {
            continue;
        }
        if let Some(time) = attribute(tag, "time") {
            date = Some(time.to_owned());
        }
        if let (Some(code), Some(rate)) = (attribute(tag, "currency"), attribute(tag, "rate")) {
            let rate: f64 = rate.parse().map_err(|_| format!("bad rate {rate:?} for {code}"))?;
            if rate <= 0.0 || !rate.is_finite() {
                return Err(format!("impossible rate {rate} for {code}"));
            }
            per_eur.insert(code.to_uppercase(), rate);
        }
    }
    let date = date.ok_or("no date in the feed")?;
    if per_eur.len() < 2 {
        return Err("no rates in the feed".into());
    }
    Ok(Rates { date, per_eur, fetched_at })
}

/// The value of `name='...'` or `name="..."` inside a tag.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let start = tag.find(&format!("{name}="))? + name.len() + 1;
    let rest = tag.get(start..)?;
    let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    rest[1..].split(quote).next()
}

/// The rates in memory and on disk.
pub struct RatesCache {
    path: Option<PathBuf>,
    current: RwLock<Option<Rates>>,
}

impl RatesCache {
    /// Loads the cache file if there is one; a missing or damaged file means no rates yet.
    pub fn open(path: Option<PathBuf>) -> Self {
        let current = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok());
        Self { path, current: RwLock::new(current) }
    }

    pub fn get(&self) -> Option<Rates> {
        self.current.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn is_stale(&self, now: i64) -> bool {
        self.get().is_none_or(|r| now - r.fetched_at > MAX_AGE_SECS)
    }

    /// Fetches with `fetch` (the feed's XML) when the cache is stale, and stores the result. A
    /// failure keeps the old rates.
    pub fn refresh(
        &self,
        now: i64,
        fetch: impl FnOnce() -> Result<String, String>,
    ) -> Result<(), String> {
        if !self.is_stale(now) {
            return Ok(());
        }
        let rates = parse_ecb(&fetch()?, now)?;
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let text = serde_json::to_string(&rates).map_err(|e| e.to_string())?;
            std::fs::write(path, text).map_err(|e| format!("cannot store the rates: {e}"))?;
        }
        *self.current.write().unwrap_or_else(|e| e.into_inner()) = Some(rates);
        Ok(())
    }
}

/// Downloads the daily feed.
pub fn fetch_ecb() -> Result<String, String> {
    // The operating system's TLS (schannel, Security.framework, OpenSSL): no bundled crypto crate.
    let tls = ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build();
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .tls_config(tls)
        .build();
    let agent: ureq::Agent = config.into();
    agent
        .get(ECB_URL)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the real feed (abbreviated).
    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope xmlns:gesmes="http://www.gesmes.org/xml/2002-08-01" xmlns="http://www.ecb.int/vocabulary/2002-08-01/eurofxref">
  <gesmes:subject>Reference rates</gesmes:subject>
  <gesmes:Sender><gesmes:name>European Central Bank</gesmes:name></gesmes:Sender>
  <Cube>
    <Cube time='2026-10-09'>
      <Cube currency='USD' rate='1.0912'/>
      <Cube currency='JPY' rate='163.45'/>
      <Cube currency="GBP" rate="0.8423"/>
    </Cube>
  </Cube>
</gesmes:Envelope>"#;

    #[test]
    fn the_feed_is_parsed_into_rates_and_a_date() {
        let rates = parse_ecb(FEED, 5).unwrap();
        assert_eq!(rates.date, "2026-10-09");
        assert_eq!(rates.rate("usd"), Some(1.0912));
        assert_eq!(rates.rate("JPY"), Some(163.45));
        assert_eq!(rates.rate("GBP"), Some(0.8423));
        assert_eq!(rates.rate("EUR"), Some(1.0));
        assert_eq!(rates.rate("XXX"), None);
        assert_eq!(rates.fetched_at, 5);
    }

    #[test]
    fn broken_feeds_are_refused() {
        assert!(parse_ecb("", 0).is_err());
        assert!(parse_ecb("<Cube time='2026-10-09'></Cube>", 0).is_err());
        assert!(parse_ecb("<Cube currency='USD' rate='1.1'/>", 0).is_err()); // no date
        assert!(
            parse_ecb("<Cube time='2026-10-09'><Cube currency='USD' rate='abc'/></Cube>", 0)
                .is_err()
        );
        assert!(
            parse_ecb("<Cube time='2026-10-09'><Cube currency='USD' rate='0'/></Cube>", 0).is_err()
        );
    }

    #[test]
    fn the_cache_fetches_when_empty_or_old_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rates.json");
        let cache = RatesCache::open(Some(path.clone()));
        assert!(cache.get().is_none() && cache.is_stale(0));

        cache.refresh(1000, || Ok(FEED.to_owned())).unwrap();
        assert_eq!(cache.get().unwrap().date, "2026-10-09");

        // Fresh enough: the fetch is not even called.
        cache.refresh(1000 + 3600, || panic!("must not fetch")).unwrap();
        // Old: fetched again; a failure keeps the old rates.
        assert!(cache.refresh(1000 + MAX_AGE_SECS + 1, || Err("offline".into())).is_err());
        assert_eq!(cache.get().unwrap().rate("usd"), Some(1.0912));

        // A new process starts from the file, offline.
        let reopened = RatesCache::open(Some(path));
        assert_eq!(reopened.get().unwrap().date, "2026-10-09");
    }

    /// Needs the network: `cargo test -p daimos-core live_feed -- --ignored`.
    #[test]
    #[ignore = "downloads the real ECB feed"]
    fn live_feed_downloads_and_parses() {
        let rates = parse_ecb(&fetch_ecb().expect("download"), 0).expect("parse");
        assert!(rates.rate("USD").is_some() && rates.rate("JPY").is_some(), "{rates:?}");
        assert!(rates.date.starts_with("20"), "{}", rates.date);
    }

    #[test]
    fn a_damaged_cache_file_means_no_rates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rates.json");
        std::fs::write(&path, "garbage").unwrap();
        assert!(RatesCache::open(Some(path)).get().is_none());
    }
}
