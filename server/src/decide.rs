//! Choosing a release: what a quality profile allows and prefers, the sizes the quality list allows, and whether a
//! release is the movie or the episodes wanted. Every release that is turned down says why, in words.
//!
//! Custom formats (TRaSH's, from [`crate::trash`]) score releases: a release below the profile's minimum score is
//! turned down, and among releases of the same quality the higher score wins, as in Radarr and Sonarr.

use crate::lantern::Release;
use crate::release::{self, Parsed};
use crate::trash::formats::{Facts, Format};
use serde::Serialize;
use serde_json::Value;

/// A quality profile read for choosing: each quality's rank (higher is better) and whether it is allowed.
#[derive(Clone, Debug, Default)]
pub struct Profile {
    /// (quality name, rank, allowed), from the lowest to the highest
    pub qualities: Vec<(String, usize, bool)>,
    /// the rank at which Marquee stops looking for better
    pub cutoff: usize,
    /// whether a file is replaced by a better one
    pub upgrades: bool,
    /// (custom format name, score) from the profile's `formatItems`, zero scores left out
    pub scores: Vec<(String, i64)>,
    /// the lowest custom format score a release may have
    pub min_score: i64,
    /// the profile's scored custom formats, ready to match (see [`Profile::with_formats`])
    pub formats: Vec<Format>,
}

impl Profile {
    /// Reads a Sonarr/Radarr-shaped profile body (`items` from the lowest quality to the highest, groups inside).
    pub fn read(v: &Value) -> Profile {
        let mut p = Profile {
            upgrades: v["upgradeAllowed"].as_bool().unwrap_or(false),
            min_score: v["minFormatScore"].as_i64().unwrap_or(0),
            scores: v["formatItems"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|f| Some((f["name"].as_str()?.to_string(), f["score"].as_i64().unwrap_or(0))))
                .filter(|(_, s)| *s != 0)
                .collect(),
            ..Profile::default()
        };
        let cutoff = v["cutoff"].as_i64().unwrap_or(-1);
        for (rank, item) in v["items"].as_array().cloned().unwrap_or_default().iter().enumerate() {
            let allowed = item["allowed"].as_bool().unwrap_or(false);
            let subs = item["items"].as_array().cloned().unwrap_or_default();
            if subs.is_empty() {
                let q = &item["quality"];
                p.qualities.push((q["name"].as_str().unwrap_or("").to_string(), rank, allowed));
                if q["id"].as_i64() == Some(cutoff) {
                    p.cutoff = rank;
                }
            } else {
                for s in &subs {
                    let q = &s["quality"];
                    p.qualities.push((q["name"].as_str().unwrap_or("").to_string(), rank, allowed));
                    if q["id"].as_i64() == Some(cutoff) {
                        p.cutoff = rank;
                    }
                }
                if item["id"].as_i64() == Some(cutoff) {
                    p.cutoff = rank;
                }
            }
        }
        p
    }

    /// The profile with the custom formats it scores, out of `all` (the kind's custom formats).
    pub fn with_formats(mut self, all: &[Format]) -> Profile {
        self.formats = all.iter().filter(|f| self.scores.iter().any(|(n, _)| *n == f.name)).cloned().collect();
        self
    }

    /// The custom formats a release matches and its total score.
    pub fn score(&self, facts: &Facts) -> (i64, Vec<String>) {
        let mut total = 0;
        let mut names = Vec::new();
        for f in self.formats.iter().filter(|f| f.matches(facts)) {
            total += self.scores.iter().find(|(n, _)| *n == f.name).map(|(_, s)| *s).unwrap_or(0);
            names.push(f.name.clone());
        }
        (total, names)
    }

    /// A quality's rank and whether the profile allows it; `None` when the profile doesn't list it.
    pub fn rank(&self, name: &str) -> Option<(usize, bool)> {
        self.qualities.iter().find(|(n, _, _)| n == name).map(|(_, r, a)| (*r, *a))
    }
}

/// The sizes the quality list allows, in MB per minute of runtime.
#[derive(Clone, Debug, Default)]
pub struct Sizes {
    /// (quality name, minimum, maximum when there is one)
    pub by_quality: Vec<(String, f64, Option<f64>)>,
}

impl Sizes {
    /// Reads the quality list of a kind (`quality_definitions` bodies).
    pub fn read(defs: &[Value]) -> Sizes {
        Sizes {
            by_quality: defs
                .iter()
                .map(|d| {
                    (d["quality"]["name"].as_str().unwrap_or("").to_string(), d["minSize"].as_f64().unwrap_or(0.0), d["maxSize"].as_f64())
                })
                .collect(),
        }
    }

    /// Why `bytes` is the wrong size for `minutes` of `quality`, or `None` when it is fine (or the runtime unknown).
    pub fn check(&self, quality: &str, bytes: i64, minutes: i64) -> Option<String> {
        if minutes <= 0 || bytes <= 0 {
            return None;
        }
        let (_, min, max) = self.by_quality.iter().find(|(n, _, _)| n == quality)?;
        let per_minute = bytes as f64 / 1_048_576.0 / minutes as f64;
        let mb = |x: f64| format!("{:.0} MB", x * minutes as f64);
        if per_minute < *min {
            return Some(format!("It's too small for {quality}: {} where at least {} is expected", mb(per_minute), mb(*min)));
        }
        match max {
            Some(m) if per_minute > *m => {
                Some(format!("It's too big for {quality}: {} where at most {} is allowed", mb(per_minute), mb(*m)))
            }
            _ => None,
        }
    }
}

/// One release looked at for an item, with what Marquee read from its name and why it was turned down.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    /// the release as Lantern found it
    pub release: Release,
    /// what its name says
    #[serde(skip)]
    pub parsed: Parsed,
    /// its quality, as the kind's quality list spells it
    pub quality: String,
    /// its rank in the profile (higher is better)
    pub rank: usize,
    /// its custom format score in the profile
    pub score: i64,
    /// the custom formats it matched
    pub formats: Vec<String>,
    /// the episodes (Marquee's ids) it holds, for TV
    pub episode_ids: Vec<i64>,
    /// why it can't be used; empty when it can
    pub rejections: Vec<String>,
}

impl Candidate {
    /// Whether it can be grabbed.
    pub fn ok(&self) -> bool {
        self.rejections.is_empty()
    }
}

/// What every release is checked against, whatever it is for.
pub struct Wanted<'a> {
    /// `movie`, `tv` or `anime`
    pub kind: &'a str,
    /// the profile
    pub profile: &'a Profile,
    /// the quality list's sizes
    pub sizes: &'a Sizes,
    /// the rank of the file there is now, when there is one
    pub current: Option<usize>,
}

/// What [`common`] found: the quality name, its rank, the custom format score and formats, and why it can't be used.
type Checked = (String, usize, i64, Vec<String>, Vec<String>);

/// The checks every release gets: quality allowed, better than the file there is, custom format score, a
/// sensible size, seeders.
fn common(w: &Wanted, r: &Release, p: &Parsed, minutes: i64) -> Checked {
    let quality = release::name_for(w.kind, p);
    let mut why = Vec::new();
    let rank = match w.profile.rank(&quality) {
        Some((rank, true)) => rank,
        Some((rank, false)) => {
            why.push(format!("{quality} isn't wanted in this quality profile"));
            rank
        }
        None => {
            why.push(format!("{quality} isn't in this quality profile"));
            0
        }
    };
    if let Some(have) = w.current {
        if !w.profile.upgrades {
            why.push("There is a file already and this profile doesn't upgrade".into());
        } else if have >= w.profile.cutoff {
            why.push("The file there is already meets the profile's cutoff".into());
        } else if rank <= have {
            why.push("It isn't better than the file there is".into());
        }
    }
    let (score, formats) = w.profile.score(&Facts::new(w.kind, &r.title, p, r.size));
    if score < w.profile.min_score {
        why.push(format!("Its custom format score is {score}, below this profile's minimum of {}", w.profile.min_score));
    }
    if let Some(s) = w.sizes.check(&quality, r.size, minutes) {
        why.push(s);
    }
    if r.protocol == "torrent" && r.seeders == Some(0) {
        why.push("No one is seeding it".into());
    }
    if r.download_url.as_deref().unwrap_or("").is_empty() && r.magnet_url.as_deref().unwrap_or("").is_empty() {
        why.push("Lantern gave no link for it".into());
    }
    (quality, rank, score, formats, why)
}

/// The best first: highest rank, then highest custom format score, then proper/repack, then most seeders, then
/// newest.
pub fn sort(c: &mut [Candidate]) {
    c.sort_by(|a, b| {
        b.ok()
            .cmp(&a.ok())
            .then(b.rank.cmp(&a.rank))
            .then(b.score.cmp(&a.score))
            .then(b.parsed.revision.cmp(&a.parsed.revision))
            .then(b.release.seeders.unwrap_or(0).cmp(&a.release.seeders.unwrap_or(0)))
            .then(a.release.age.cmp(&b.release.age))
    });
}

/// Checks releases for a movie (`titles`: its title and original title, `year`, `minutes` of runtime).
pub fn for_movie(w: &Wanted, releases: Vec<Release>, titles: &[&str], year: i64, minutes: i64) -> Vec<Candidate> {
    let names: Vec<String> = titles.iter().map(|t| release::clean(t)).filter(|t| !t.is_empty()).collect();
    let mut out: Vec<Candidate> = releases
        .into_iter()
        .map(|r| {
            let p = release::parse(&r.title);
            let (quality, rank, score, formats, mut why) = common(w, &r, &p, minutes);
            if !names.contains(&release::clean(&p.title)) {
                why.insert(0, format!("It's for “{}”, not this movie", p.title));
            } else if p.year > 0 && year > 0 && (p.year - year).abs() > 1 {
                why.insert(0, format!("It's from {}, not {year}", p.year));
            }
            if p.season.is_some() {
                why.insert(0, "It's a TV episode".into());
            }
            Candidate { release: r, parsed: p, quality, rank, score, formats, episode_ids: vec![], rejections: why }
        })
        .collect();
    sort(&mut out);
    out
}

/// An episode a TV search wants: Marquee's id, season, number, absolute number and runtime.
#[derive(Clone, Debug)]
pub struct WantedEpisode {
    /// Marquee's id
    pub id: i64,
    /// the season
    pub season: i64,
    /// the number in the season
    pub episode: i64,
    /// the absolute number (anime)
    pub absolute: Option<i64>,
    /// minutes
    pub runtime: i64,
    /// the rank of the file it has now
    pub current: Option<usize>,
}

/// Checks releases for some episodes of one series (`titles`: the series' names). A release must hold only
/// episodes that are wanted (a season pack: the season's wanted episodes, and they must be all its episodes).
pub fn for_episodes(
    w: &Wanted,
    releases: Vec<Release>,
    titles: &[&str],
    wanted: &[WantedEpisode],
    season_sizes: &[(i64, usize)],
) -> Vec<Candidate> {
    let names: Vec<String> = titles.iter().map(|t| release::clean(t)).filter(|t| !t.is_empty()).collect();
    let mut out: Vec<Candidate> = releases
        .into_iter()
        .map(|r| {
            let p = release::parse(&r.title);
            let mut why = Vec::new();
            let mut ids = Vec::new();
            let held: Vec<&WantedEpisode> = match (p.season, p.full_season, p.absolute) {
                (Some(s), true, _) => {
                    let in_season: Vec<&WantedEpisode> = wanted.iter().filter(|e| e.season == s).collect();
                    let all = season_sizes.iter().find(|(n, _)| *n == s).map(|(_, c)| *c).unwrap_or(0);
                    if in_season.is_empty() {
                        why.push(format!("Season {s} isn't wanted"));
                    } else if in_season.len() < all {
                        why.push(format!("It's all of season {s}, but only {} of its episodes are wanted", in_season.len()));
                    }
                    in_season
                }
                (Some(s), false, _) => {
                    let found: Vec<&WantedEpisode> = wanted.iter().filter(|e| e.season == s && p.episodes.contains(&e.episode)).collect();
                    if found.len() < p.episodes.len() {
                        why.push("It holds episodes that aren't wanted".into());
                    }
                    found
                }
                (None, _, Some(a)) => {
                    let found: Vec<&WantedEpisode> = wanted.iter().filter(|e| e.absolute == Some(a)).collect();
                    if found.is_empty() {
                        why.push(format!("Episode {a} isn't wanted"));
                    }
                    found
                }
                _ => {
                    why.push("Its name doesn't say which episode it is".into());
                    vec![]
                }
            };
            ids.extend(held.iter().map(|e| e.id));
            let minutes: i64 = held.iter().map(|e| e.runtime).sum();
            let current = held.iter().map(|e| e.current).min().flatten();
            let w2 = Wanted { current, ..*w };
            let (quality, rank, score, formats, more) = common(&w2, &r, &p, minutes);
            why.extend(more);
            if !names.contains(&release::clean(&p.title)) {
                why.insert(0, format!("It's for “{}”, not this series", p.title));
            }
            Candidate { release: r, parsed: p, quality, rank, score, formats, episode_ids: ids, rejections: why }
        })
        .collect();
    sort(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile() -> Profile {
        Profile::read(&json!({
            "upgradeAllowed": true, "cutoff": 7,
            "items": [
                {"quality": {"id": 1, "name": "SDTV"}, "items": [], "allowed": false},
                {"quality": {"id": 4, "name": "HDTV-720p"}, "items": [], "allowed": true},
                {"name": "WEB 1080p", "id": 1001, "allowed": true, "items": [
                    {"quality": {"id": 3, "name": "WEBDL-1080p"}, "items": [], "allowed": true},
                    {"quality": {"id": 15, "name": "WEBRip-1080p"}, "items": [], "allowed": true}]},
                {"quality": {"id": 7, "name": "Bluray-1080p"}, "items": [], "allowed": true}
            ]
        }))
    }

    fn rel(title: &str, gb: f64, seeders: i64) -> Release {
        Release {
            title: title.into(),
            size: (gb * 1_073_741_824.0) as i64,
            protocol: "torrent".into(),
            seeders: Some(seeders),
            download_url: Some("http://lantern/1/download?x".into()),
            ..Release::default()
        }
    }

    #[test]
    fn profiles_rank_groups_together() {
        let p = profile();
        assert_eq!(p.rank("WEBDL-1080p"), p.rank("WEBRip-1080p"));
        assert_eq!(p.rank("SDTV"), Some((0, false)));
        assert_eq!(p.cutoff, 3);
    }

    #[test]
    fn the_best_allowed_release_comes_first_and_the_rest_say_why() {
        let p = profile();
        let sizes = Sizes::read(&[json!({"quality": {"name": "Bluray-1080p"}, "minSize": 4.0, "maxSize": 155.0})]);
        let w = Wanted { kind: "movie", profile: &p, sizes: &sizes, current: None };
        let found = for_movie(
            &w,
            vec![
                rel("The.Matrix.1999.720p.HDTV.x264-GRP", 4.0, 10),
                rel("The.Matrix.1999.1080p.BluRay.x264-SPARKS", 10.0, 50),
                rel("The.Matrix.1999.1080p.BluRay.x264-TINY", 0.1, 50),
                rel("The.Matrix.Reloaded.2003.1080p.BluRay.x264-GRP", 10.0, 50),
                rel("The.Matrix.1999.480p.SDTV-GRP", 1.0, 5),
                rel("The.Matrix.1999.1080p.WEB-DL-DEAD", 8.0, 0),
            ],
            &["The Matrix"],
            1999,
            136,
        );
        assert_eq!(found[0].release.title, "The.Matrix.1999.1080p.BluRay.x264-SPARKS");
        assert_eq!(found[1].release.title, "The.Matrix.1999.720p.HDTV.x264-GRP");
        let why = |t: &str| found.iter().find(|c| c.release.title.contains(t)).unwrap().rejections.join("; ");
        assert!(why("TINY").starts_with("It's too small for Bluray-1080p"), "{}", why("TINY"));
        assert!(why("Reloaded").contains("not this movie"));
        assert!(why("SDTV").contains("isn't wanted"));
        assert_eq!(why("DEAD"), "No one is seeding it");
        let w = Wanted { current: Some(3), ..w };
        let again = for_movie(&w, vec![rel("The.Matrix.1999.1080p.BluRay.x264-SPARKS", 10.0, 50)], &["The Matrix"], 1999, 136);
        assert!(!again[0].ok(), "the cutoff is met");
    }

    #[test]
    fn custom_formats_score_and_order_releases() {
        let mut body = json!({
            "upgradeAllowed": true, "cutoff": 7, "minFormatScore": 0,
            "items": [{"quality": {"id": 7, "name": "Bluray-1080p"}, "items": [], "allowed": true}],
            "formatItems": [{"format": 1, "name": "HD Bluray Tier 01", "score": 1800}, {"format": 2, "name": "LQ", "score": -10000}]
        });
        let tier = Format::read(&json!({"name": "HD Bluray Tier 01", "specifications": [
            {"name": "g", "implementation": "ReleaseGroupSpecification", "required": false, "fields": [{"name": "value", "value": "^(DON)$"}]}]}));
        let lq = Format::read(&json!({"name": "LQ", "specifications": [
            {"name": "g", "implementation": "ReleaseGroupSpecification", "required": false, "fields": [{"name": "value", "value": "^(YIFY)$"}]}]}));
        let p = Profile::read(&body).with_formats(&[tier.clone(), lq.clone()]);
        let sizes = Sizes::default();
        let w = Wanted { kind: "movie", profile: &p, sizes: &sizes, current: None };
        let found = for_movie(
            &w,
            vec![rel("The.Matrix.1999.1080p.BluRay.x264-OTHER", 10.0, 90), rel("The.Matrix.1999.1080p.BluRay.x264-DON", 10.0, 5)],
            &["The Matrix"],
            1999,
            136,
        );
        assert_eq!(found[0].release.title, "The.Matrix.1999.1080p.BluRay.x264-DON", "the higher score wins over seeders");
        assert_eq!((found[0].score, found[0].formats.clone()), (1800, vec!["HD Bluray Tier 01".to_string()]));
        body["minFormatScore"] = json!(0);
        let p = Profile::read(&body).with_formats(&[tier, lq]);
        let w = Wanted { kind: "movie", profile: &p, sizes: &sizes, current: None };
        let found = for_movie(&w, vec![rel("The.Matrix.1999.1080p.BluRay.x264-YIFY", 10.0, 90)], &["The Matrix"], 1999, 136);
        assert_eq!(found[0].rejections, vec!["Its custom format score is -10000, below this profile's minimum of 0"]);
    }

    #[test]
    fn episodes_and_season_packs_are_matched() {
        let p = profile();
        let sizes = Sizes::default();
        let w = Wanted { kind: "tv", profile: &p, sizes: &sizes, current: None };
        let ep = |id, season, episode| WantedEpisode { id, season, episode, absolute: None, runtime: 30, current: None };
        let wanted = [ep(2, 1, 1), ep(3, 1, 2)];
        let found = for_episodes(
            &w,
            vec![
                rel("The.Bear.S01E02.1080p.WEB-DL-NTb", 1.0, 9),
                rel("The.Bear.S01.1080p.WEB-DL-NTb", 2.0, 9),
                rel("The.Bear.S02E01.1080p.WEB-DL-NTb", 1.0, 9),
                rel("The.Bear.S01E02E03.1080p.WEB-DL-NTb", 1.0, 9),
            ],
            &["The Bear"],
            &wanted,
            &[(1, 2)],
        );
        let get = |t: &str| found.iter().find(|c| c.release.title == t).unwrap();
        assert_eq!(get("The.Bear.S01.1080p.WEB-DL-NTb").episode_ids, vec![2, 3]);
        assert!(get("The.Bear.S01.1080p.WEB-DL-NTb").ok());
        assert_eq!(get("The.Bear.S01E02.1080p.WEB-DL-NTb").episode_ids, vec![3]);
        assert!(!get("The.Bear.S02E01.1080p.WEB-DL-NTb").ok());
        assert_eq!(get("The.Bear.S01E02E03.1080p.WEB-DL-NTb").rejections, vec!["It holds episodes that aren't wanted"]);
        let only_one = for_episodes(&w, vec![rel("The.Bear.S01.1080p.WEB-DL-NTb", 2.0, 9)], &["The Bear"], &wanted[..1], &[(1, 2)]);
        assert!(only_one[0].rejections[0].contains("only 1 of its episodes"));
    }
}
