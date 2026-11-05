//! Reading a release's name: the title, year, season and episodes, the quality (source and resolution), the group,
//! and whether it is a proper or repack. Qualities are named as Sonarr names them; [`name_for`] gives Radarr's name
//! for movies, and the quality list of each kind (`quality_definitions`) gives the id.

use regex::Regex;
use std::sync::LazyLock;

/// What a release's name says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    /// the title words before the year or episode number, with dots and underscores as spaces
    pub title: String,
    /// the year, 0 when the name has none
    pub year: i64,
    /// the season, `None` for a movie
    pub season: Option<i64>,
    /// the episodes in the season (empty for a whole season or a movie)
    pub episodes: Vec<i64>,
    /// an absolute episode number (anime style `Title - 012`)
    pub absolute: Option<i64>,
    /// true for a whole season (`S01` with no episode)
    pub full_season: bool,
    /// the quality's name, as Sonarr spells it
    pub quality_name: &'static str,
    /// the resolution in lines, 0 when unknown
    pub resolution: i64,
    /// 1, or 2 and more for a proper, repack or real
    pub revision: i64,
    /// the release group, "" when unknown
    pub group: String,
}

/// `SxxEyy` with more episodes after it (`E01E02`, `E01-E02`, `E01-02`).
static SXE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bS(\d{1,3})[ ._-]?E(\d{1,4})((?:[ ._-]?-?E?\d{1,4})*)").unwrap());
/// `1x02`.
static NXE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{2,3})\b").unwrap());
/// A whole season: `S01`, `Season 1`.
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:S(\d{1,3})|Season[ ._-]?(\d{1,3}))\b").unwrap());
/// Anime style absolute numbers: `Title - 012`.
static ABSOLUTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ ._]-[ ._](\d{2,4})(?:v\d)?\b").unwrap());
/// A year from 1900 to 2099.
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(\[ ._]((?:19|20)\d{2})[)\] ._]").unwrap());
/// The resolution.
static RES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(2160p|4k|uhd|1080[pi]|720p|576p|480p)\b").unwrap());
/// The group after the last dash, before an extension.
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-([A-Za-z0-9]+)(?:\[[^\]]*\])?(?:\.(?:mkv|mp4|avi|m4v|ts))?$").unwrap());
/// Anime style group at the start: `[Group] Title`.
static LEAD_GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([^\]]+)\]").unwrap());

/// True when `name` holds one of `words` as a whole word (any case).
fn has(name: &str, words: &[&str]) -> bool {
    let lower = name.to_lowercase();
    let parts: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').collect();
    words.iter().any(|w| {
        let w = w.to_lowercase();
        parts.iter().any(|p| *p == w) || (w.contains(['-', ' ', '.']) && lower.contains(&w))
    })
}

/// The quality's name (Sonarr's spelling) and the resolution a release name says.
pub fn quality_of(name: &str) -> (&'static str, i64) {
    let res = RES.captures(name).map(|c| c[1].to_lowercase()).unwrap_or_default();
    let r = match res.as_str() {
        "2160p" | "4k" | "uhd" => 2160,
        "1080p" | "1080i" => 1080,
        "720p" => 720,
        "576p" => 576,
        "480p" => 480,
        _ => 0,
    };
    let remux = has(name, &["remux"]);
    let bluray = has(name, &["bluray", "blu-ray", "bdrip", "brrip", "bd", "bdremux"]);
    let webdl = has(name, &["web-dl", "webdl", "amzn", "nf", "dsnp", "hmax", "atvp", "pcok", "hulu", "itunes"]) && !has(name, &["webrip"]);
    let webrip = has(name, &["webrip", "web-rip"]);
    let web = has(name, &["web"]);
    let hdtv = has(name, &["hdtv", "pdtv", "sdtv", "dsr", "tvrip"]);
    let dvd = has(name, &["dvdrip", "dvd", "dvd9", "dvd5", "ntsc", "pal"]);
    let q = if has(name, &["cam", "camrip", "hdcam"]) {
        "CAM"
    } else if has(name, &["ts", "telesync", "hdts", "pdvd"]) {
        "TELESYNC"
    } else if has(name, &["telecine", "tc", "hdtc"]) {
        "TELECINE"
    } else if has(name, &["dvdscr", "screener", "scr"]) {
        "DVDSCR"
    } else if remux {
        if r == 2160 {
            "Bluray-2160p Remux"
        } else {
            "Bluray-1080p Remux"
        }
    } else if bluray {
        match r {
            2160 => "Bluray-2160p",
            1080 => "Bluray-1080p",
            720 => "Bluray-720p",
            576 => "Bluray-576p",
            _ => "Bluray-480p",
        }
    } else if webrip {
        match r {
            2160 => "WEBRip-2160p",
            1080 => "WEBRip-1080p",
            720 => "WEBRip-720p",
            _ => "WEBRip-480p",
        }
    } else if webdl || web {
        match r {
            2160 => "WEBDL-2160p",
            1080 => "WEBDL-1080p",
            720 => "WEBDL-720p",
            _ => "WEBDL-480p",
        }
    } else if hdtv {
        match r {
            2160 => "HDTV-2160p",
            1080 => "HDTV-1080p",
            720 => "HDTV-720p",
            _ => "SDTV",
        }
    } else if dvd {
        "DVD"
    } else {
        match r {
            2160 => "HDTV-2160p",
            1080 => "HDTV-1080p",
            720 => "HDTV-720p",
            480 | 576 => "SDTV",
            _ => "Unknown",
        }
    };
    (q, r)
}

/// The words of a title: dots and underscores as spaces, trimmed of separators.
fn title_words(s: &str) -> String {
    let t = s.replace(['.', '_'], " ");
    let t = LEAD_GROUP.replace(&t, "");
    t.trim_matches(|c: char| c == ' ' || c == '-' || c == '(' || c == '[').split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Reads a release name.
pub fn parse(name: &str) -> Parsed {
    let name = name.trim();
    let (quality_name, resolution) = quality_of(name);
    let mut p = Parsed { quality_name, resolution, revision: 1, ..Parsed::default() };
    if has(name, &["proper", "repack", "rerip", "real"]) {
        p.revision = 2;
    }
    p.group =
        GROUP.captures(name).map(|c| c[1].to_string()).or_else(|| LEAD_GROUP.captures(name).map(|c| c[1].to_string())).unwrap_or_default();
    let mut cut = name.len();
    if let Some(c) = SXE.captures(name) {
        let season = c[1].parse().unwrap_or(0);
        let first: i64 = c[2].parse().unwrap_or(0);
        let mut eps = vec![first];
        let more = Regex::new(r"\d{1,4}").unwrap();
        let mut last = first;
        let rest = c.get(3).map(|m| m.as_str()).unwrap_or("");
        let ranged = rest.trim_start_matches([' ', '.', '_']).starts_with('-');
        for m in more.find_iter(rest) {
            let n: i64 = m.as_str().parse().unwrap_or(0);
            if n > last && n - last < 50 {
                if ranged {
                    eps.extend(last + 1..=n);
                } else {
                    eps.push(n);
                }
                last = n;
            }
        }
        p.season = Some(season);
        p.episodes = eps;
        cut = c.get(0).unwrap().start();
    } else if let Some(c) = NXE.captures(name) {
        p.season = c[1].parse().ok();
        p.episodes = vec![c[2].parse().unwrap_or(0)];
        cut = c.get(0).unwrap().start();
    } else if let Some(c) = SEASON.captures(name) {
        p.season = c.get(1).or(c.get(2)).and_then(|m| m.as_str().parse().ok());
        p.full_season = true;
        cut = c.get(0).unwrap().start();
    } else if let Some(c) = ABSOLUTE.captures(name) {
        p.absolute = c[1].parse().ok();
        cut = c.get(0).unwrap().start();
    }
    let padded = format!("{name} ");
    let episodic = p.season.is_some() || p.absolute.is_some();
    // the last year (before the episode number for TV): a title can hold a year, `Blade Runner 2049 (2017)`
    let year = YEAR.captures_iter(&padded).filter_map(|c| c.get(1)).filter(|m| !episodic || m.start() < cut).last();
    if let Some(m) = year {
        p.year = m.as_str().parse().unwrap_or(0);
        if !episodic {
            cut = m.start();
        }
    }
    if p.season.is_none() && p.absolute.is_none() && p.year == 0 {
        cut = RES.find(name).map(|m| m.start()).unwrap_or(cut);
    }
    p.title = title_words(&name[..cut.min(name.len())]);
    if p.year > 0 && p.season.is_some() {
        let y = p.year.to_string();
        if let Some(t) = p.title.strip_suffix(&y) {
            p.title = t.trim_end_matches([' ', '(']).to_string();
        }
    }
    p
}

/// A quality's name as `kind`'s quality list spells it (movies: Radarr's names).
pub fn name_for(kind: &str, sonarr_name: &str) -> String {
    match (kind, sonarr_name) {
        ("movie", "Bluray-1080p Remux") => "Remux-1080p".into(),
        ("movie", "Bluray-2160p Remux") => "Remux-2160p".into(),
        _ => sonarr_name.into(),
    }
}

/// A title reduced to lowercase letters and digits, for comparing a release's title with a movie or series.
pub fn clean(t: &str) -> String {
    let t = t.to_lowercase().replace('&', "and");
    let words: Vec<&str> = t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let words: Vec<&str> = match words.first() {
        Some(&"the") | Some(&"a") | Some(&"an") if words.len() > 1 => words[1..].to_vec(),
        _ => words,
    };
    words.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movies_are_read() {
        let p = parse("The.Matrix.1999.1080p.BluRay.x264-SPARKS");
        assert_eq!((p.title.as_str(), p.year, p.quality_name, p.group.as_str()), ("The Matrix", 1999, "Bluray-1080p", "SPARKS"));
        assert_eq!(p.season, None);
        let p = parse("Dune.Part.Two.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR.H.265-FLUX");
        assert_eq!((p.title.as_str(), p.year, p.quality_name), ("Dune Part Two", 2024, "WEBDL-2160p"));
        let p = parse("Blade Runner 2049 (2017) 1080p BluRay REMUX AVC DTS-HD MA 7.1-FGT");
        assert_eq!((p.title.as_str(), p.year, p.quality_name), ("Blade Runner 2049", 2017, "Bluray-1080p Remux"));
        assert_eq!(name_for("movie", p.quality_name), "Remux-1080p");
        let p = parse("Oppenheimer.2023.REPACK.720p.WEBRip.x264-GalaxyRG");
        assert_eq!((p.quality_name, p.revision), ("WEBRip-720p", 2));
        assert_eq!(parse("Some.Movie.2024.HDCAM.x264-XYZ").quality_name, "CAM");
    }

    #[test]
    fn episodes_are_read() {
        let p = parse("The.Bear.S01E02.Hands.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb");
        assert_eq!((p.title.as_str(), p.season, p.episodes.clone(), p.quality_name), ("The Bear", Some(1), vec![2], "WEBDL-1080p"));
        assert_eq!(parse("Show.Name.S02E01E02.720p.HDTV.x264-GRP").episodes, vec![1, 2]);
        assert_eq!(parse("Show.Name.S02E01-E03.720p.HDTV.x264-GRP").episodes, vec![1, 2, 3]);
        let p = parse("The Bear S02 1080p WEB-DL DDP5.1 H 264-NTb");
        assert_eq!((p.season, p.full_season, p.episodes.len()), (Some(2), true, 0));
        let p = parse("Doctor.Who.2005.S01E01.Rose.720p.BluRay-GRP");
        assert_eq!((p.title.as_str(), p.year, p.season), ("Doctor Who", 2005, Some(1)));
        let p = parse("[SubsPlease] Frieren - 12 (1080p) [ABCD1234].mkv");
        assert_eq!((p.title.as_str(), p.absolute, p.resolution, p.group.as_str()), ("Frieren", Some(12), 1080, "SubsPlease"));
        assert_eq!(parse("show.name.1x05.hdtv-lol").episodes, vec![5]);
    }

    #[test]
    fn titles_compare_without_articles_and_marks() {
        assert_eq!(clean("The Matrix"), clean("Matrix"));
        assert_eq!(clean("Marvel's Agents of S.H.I.E.L.D."), "marvelsagentsofshield");
        assert_eq!(clean("Law & Order"), clean("Law and Order"));
    }
}
