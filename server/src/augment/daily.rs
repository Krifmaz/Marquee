//! Daily series: the TheTVDB ids Sonarr's services list as shows named by air date (talk shows, news). A port
//! of Sonarr's `DataAugmentation/DailySeries` (`DailySeriesDataProxy`, `DailySeriesService`) at the commit in
//! `UPSTREAM.lock`.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;

/// Where Sonarr's services keep the daily series list.
pub const DAILY_URL: &str = "https://services.sonarr.tv/v1/dailyseries";

/// How long a fetched list is used before it is fetched again (Sonarr's one hour).
const KEEP: Duration = Duration::from_secs(3600);

/// Reads the list's answer (`[{"tvdbId": 1}, ...]`) as TheTVDB ids.
pub fn ids_from(answer: &Value) -> Vec<i64> {
    answer.as_array().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|d| d["tvdbId"].as_i64()).collect()
}

/// Sonarr's `DailySeriesService`: whether a series is a daily show, with the list kept for an hour.
#[derive(Default)]
pub struct DailySeries {
    /// The last list and when it was fetched.
    cache: Mutex<Option<(Instant, Vec<i64>)>>,
}

impl DailySeries {
    /// A service that has fetched nothing yet.
    pub fn new() -> DailySeries {
        DailySeries::default()
    }

    /// Whether TheTVDB id `tvdb_id` is on the daily series list, fetching the list when it is older than an hour.
    pub async fn is_daily_series(&self, http: &reqwest::Client, tvdb_id: i64) -> bool {
        let fresh =
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).as_ref().filter(|(at, _)| at.elapsed() < KEEP).map(|(_, l)| l.clone());
        let list = match fresh {
            Some(l) => l,
            None => {
                let l = fetch(http).await;
                self.set(l.clone());
                l
            }
        };
        list.contains(&tvdb_id)
    }

    /// Keeps `list` as the list fetched now.
    pub fn set(&self, list: Vec<i64>) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), list));
    }
}

/// Fetches the list; an empty one when it can't be fetched (Sonarr's `GetDailySeriesIds`).
async fn fetch(http: &reqwest::Client) -> Vec<i64> {
    let answer = async {
        let r = http.get(DAILY_URL).timeout(Duration::from_secs(30)).send().await.ok()?;
        r.status().is_success().then_some(())?;
        r.json::<Value>().await.ok()
    }
    .await;
    match answer {
        Some(v) => ids_from(&v),
        None => {
            tracing::warn!("Failed to get Daily Series");
            Vec::new()
        }
    }
}
