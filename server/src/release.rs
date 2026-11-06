//! Reading a release's name: the title, year, season and episodes, the quality (source and resolution), the group,
//! and whether it is a proper or repack. Everything comes from the ports of Sonarr's and Radarr's parsers
//! ([`crate::parser`]): movie names are read by Radarr's `Parser.cs` ([`parse_movie`]), series names by Sonarr's
//! ([`parse_series`]); `quality_name` is Sonarr's quality reading and `movie_quality_name` Radarr's, [`name_for`]
//! picks the one for a kind, and the quality list of each kind (`quality_definitions`) gives the id.

use crate::parser::{episode, movie, quality_movie, quality_tv, release_group_movie};

/// What a release's name says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parsed {
    /// the movie or series title, "" when the name can't be read
    pub title: String,
    /// the year, 0 when the name has none
    pub year: i64,
    /// the season, `None` for a movie, a daily episode or anime numbered only by absolute episode
    pub season: Option<i64>,
    /// the episodes in the season (empty for a whole season or a movie)
    pub episodes: Vec<i64>,
    /// the first absolute episode number (anime style `Title - 012`)
    pub absolute: Option<i64>,
    /// every absolute episode number (`Title - 01-12`)
    pub absolutes: Vec<i64>,
    /// true for a whole season (`S01` with no episode)
    pub full_season: bool,
    /// the air date of a daily episode (`yyyy-MM-dd`), "" for others
    pub air_date: String,
    /// the quality's name, as Sonarr's parser reads it
    pub quality_name: &'static str,
    /// the quality's name, as Radarr's parser reads it (for movies)
    pub movie_quality_name: &'static str,
    /// the resolution in lines, 0 when unknown
    pub resolution: i64,
    /// Sonarr's revision version: 1, 2 for a proper or repack, more for `REPACK2` or anime `v3`
    pub revision: i64,
    /// the release group, "" when unknown
    pub group: String,
}

/// The quality's name (Sonarr's parser) and the resolution a release name says.
pub fn quality_of(name: &str) -> (&'static str, i64) {
    let q = quality_tv::parse_quality(name);
    let r = match quality_tv::parse_resolution(name.replace('_', " ").trim()) {
        quality_tv::Resolution::R360p => 360,
        quality_tv::Resolution::R480p => 480,
        quality_tv::Resolution::R540p => 540,
        quality_tv::Resolution::R576p => 576,
        quality_tv::Resolution::R720p => 720,
        quality_tv::Resolution::R1080p => 1080,
        quality_tv::Resolution::R2160p => 2160,
        quality_tv::Resolution::Unknown => 0,
    };
    (q.quality.name, r)
}

/// The qualities of a name, before its title is read.
fn qualities(name: &str) -> Parsed {
    let (quality_name, resolution) = quality_of(name);
    let revision = quality_tv::parse_quality(name).revision.version;
    let movie_quality_name = quality_movie::parse_quality(name).quality.name;
    Parsed { quality_name, movie_quality_name, resolution, revision, ..Parsed::default() }
}

/// Copies what Sonarr's parser read about seasons and episodes.
fn take_episodes(p: &mut Parsed, e: &episode::ParsedEpisodeInfo) {
    if !e.episode_numbers.is_empty() || e.full_season {
        p.season = Some(i64::from(e.season_number));
        p.episodes = e.episode_numbers.iter().map(|&n| i64::from(n)).collect();
        p.full_season = e.full_season;
    }
}

/// Reads a movie release name with Radarr's parser (which knows nothing of seasons, so `season` stays `None`).
pub fn parse_movie(name: &str) -> Parsed {
    let name = name.trim();
    let mut p = qualities(name);
    match movie::parse_movie_title(name, false) {
        Some(m) => {
            p.title = m.primary_movie_title().unwrap_or_default().to_string();
            p.year = i64::from(m.year);
            p.group = m.release_group.unwrap_or_default();
        }
        None => p.group = release_group_movie::parse_release_group(name).unwrap_or_default(),
    }
    p
}

/// Reads a series release or file name with Sonarr's parser.
pub fn parse_series(name: &str) -> Parsed {
    let name = name.trim();
    let mut p = qualities(name);
    match episode::parse_title(name) {
        Some(e) => {
            p.title = e.series_title_info.title_without_year.clone();
            p.year = i64::from(e.series_title_info.year);
            p.group = e.release_group.clone().unwrap_or_default();
            p.absolutes = e.absolute_episode_numbers.iter().map(|&n| i64::from(n)).collect();
            p.absolute = p.absolutes.first().copied();
            p.air_date = e.air_date.clone();
            take_episodes(&mut p, &e);
        }
        None => p.group = episode::parse_release_group(name).unwrap_or_default(),
    }
    p
}

/// Reads a release name the way `kind` (`movie`, `tv`, `anime`) is read.
pub fn parse_for(kind: &str, name: &str) -> Parsed {
    if kind == "movie" {
        parse_movie(name)
    } else {
        parse_series(name)
    }
}

/// A release's quality name as `kind`'s quality list spells it: Radarr's reading for movies, Sonarr's otherwise.
pub fn name_for(kind: &str, p: &Parsed) -> String {
    if kind == "movie" {
        p.movie_quality_name.into()
    } else {
        p.quality_name.into()
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
        let p = parse_movie("The.Matrix.1999.1080p.BluRay.x264-SPARKS");
        assert_eq!((p.title.as_str(), p.year, p.quality_name, p.group.as_str()), ("The Matrix", 1999, "Bluray-1080p", "SPARKS"));
        assert_eq!(p.season, None);
        let p = parse_movie("Dune.Part.Two.2024.2160p.WEB-DL.DDP5.1.Atmos.DV.HDR.H.265-FLUX");
        assert_eq!((p.title.as_str(), p.year, p.quality_name), ("Dune Part Two", 2024, "WEBDL-2160p"));
        let p = parse_movie("Blade.Runner.2049.2017.1080p.BluRay.REMUX.AVC.DTS-HD.MA.7.1-FGT");
        assert_eq!((p.title.as_str(), p.year, p.quality_name), ("Blade Runner 2049", 2017, "Bluray-1080p Remux"));
        assert_eq!(name_for("movie", &p), "Remux-1080p");
        let p = parse_movie("Oppenheimer.2023.REPACK.720p.WEBRip.x264-GalaxyRG");
        assert_eq!((p.quality_name, p.revision), ("WEBRip-720p", 2));
        assert_eq!(parse_movie("Some.Movie.2024.HDCAM.x264-XYZ").movie_quality_name, "CAM");
        assert_eq!(parse_movie("not a release").title, "");
    }

    #[test]
    fn episodes_are_read() {
        let p = parse_series("The.Bear.S01E02.Hands.1080p.DSNP.WEB-DL.DDP5.1.H.264-NTb");
        assert_eq!((p.title.as_str(), p.season, p.episodes.clone(), p.quality_name), ("The Bear", Some(1), vec![2], "WEBDL-1080p"));
        assert_eq!(parse_series("Show.Name.S02E01E02.720p.HDTV.x264-GRP").episodes, vec![1, 2]);
        assert_eq!(parse_series("Show.Name.S02E01-E03.720p.HDTV.x264-GRP").episodes, vec![1, 2, 3]);
        let p = parse_series("The Bear S02 1080p WEB-DL DDP5.1 H 264-NTb");
        assert_eq!((p.season, p.full_season, p.episodes.len()), (Some(2), true, 0));
        let p = parse_series("Doctor.Who.2005.S01E01.Rose.720p.BluRay-GRP");
        assert_eq!((p.title.as_str(), p.year, p.season), ("Doctor Who", 2005, Some(1)));
        let p = parse_series("[SubsPlease] Frieren - 12 (1080p) [ABCD1234].mkv");
        assert_eq!(
            (p.title.as_str(), p.absolute, p.season, p.resolution, p.group.as_str()),
            ("Frieren", Some(12), None, 1080, "SubsPlease")
        );
        assert_eq!(parse_series("[Group] Frieren - 01-04 [1080p]").absolutes, vec![1, 2, 3, 4]);
        assert_eq!(parse_series("show.name.1x05.hdtv-lol").episodes, vec![5]);
        assert_eq!(parse_series("The.Daily.Show.2024.03.05.720p.WEB-GRP").air_date, "2024-03-05");
    }

    #[test]
    fn titles_compare_without_articles_and_marks() {
        assert_eq!(clean("The Matrix"), clean("Matrix"));
        assert_eq!(clean("Marvel's Agents of S.H.I.E.L.D."), "marvelsagentsofshield");
        assert_eq!(clean("Law & Order"), clean("Law and Order"));
    }
}
