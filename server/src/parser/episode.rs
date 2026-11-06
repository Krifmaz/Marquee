//! Sonarr's `Parser/Parser`, `Parser/SceneChecker` and `Parser/Model/ParsedEpisodeInfo`: reading series, season,
//! episode, absolute (anime) and air-date numbers from a release or file name. Port of the files at the Sonarr
//! commit in `UPSTREAM.lock`; the expressions are the upstream ones, unchanged, and run on [`super::clr_regex`]
//! because the parser reads every capture of a repeated group (`S01E01E02E03`).

use super::clr_regex::{ClrMatch, ClrRegex};
use super::language::Language;
use super::language_tv;
use super::quality::{tv, QualityModel, TvQuality};
use super::quality_tv;
use super::release_group_movie::remove_file_extension;
use super::text::{byte_at_utf16, dotnet_int, dotnet_long, remove_accent, utf16_len};
use chrono::{Datelike, NaiveDate};
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// What kind of release a parsed name is (`ReleaseType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseType {
    /// not known
    Unknown,
    /// one episode
    SingleEpisode,
    /// several episodes
    MultiEpisode,
    /// a whole season
    SeasonPack,
}

/// The series name and what it holds (`SeriesTitleInfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SeriesTitleInfo {
    /// the series name as read
    pub title: String,
    /// the name without a year at its end
    pub title_without_year: String,
    /// the year at the end of the name, 0 without one
    pub year: i32,
    /// every name the release gives (`A (B)`, `A | B`, `A AKA B`, `A / B / S01E01`); empty when it gives one
    pub all_titles: Vec<String>,
}

/// What a release or file name says about the episodes in it (`ParsedEpisodeInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedEpisodeInfo {
    /// the name without its file extension
    pub release_title: String,
    /// the series name
    pub series_title: String,
    /// the series name and the years and other names in it
    pub series_title_info: SeriesTitleInfo,
    /// the quality
    pub quality: QualityModel<TvQuality>,
    /// the season, 0 for specials
    pub season_number: i32,
    /// the episode numbers within the season
    pub episode_numbers: Vec<i32>,
    /// the absolute episode numbers (anime)
    pub absolute_episode_numbers: Vec<i32>,
    /// absolute numbers of specials that sit between episodes (`12.5`)
    pub special_absolute_episode_numbers: Vec<f64>,
    /// the air date of a daily episode (`yyyy-MM-dd`), empty for others
    pub air_date: String,
    /// the languages
    pub languages: Vec<Language>,
    /// a whole season
    pub full_season: bool,
    /// part of a season (`S01 Part 1`)
    pub is_partial_season: bool,
    /// more than one season
    pub is_multi_season: bool,
    /// a season's extras or subtitles, not its episodes
    pub is_season_extra: bool,
    /// half of an episode (`S01E05a`)
    pub is_split_episode: bool,
    /// a mini-series without a season number, read as season 1
    pub is_mini_series: bool,
    /// a special
    pub special: bool,
    /// the group that made the release
    pub release_group: Option<String>,
    /// the hash at the end of an anime release (`[ABCD1234]`), empty without one
    pub release_hash: String,
    /// which part of a partial season
    pub season_part: i32,
    /// the name after the episode numbers, where languages and quality are read
    pub release_tokens: String,
    /// which part of a daily episode (`2018.04.28.Part.2`)
    pub daily_part: Option<i32>,
}

impl ParsedEpisodeInfo {
    /// An empty result for a release name.
    pub fn new(release_title: &str) -> ParsedEpisodeInfo {
        ParsedEpisodeInfo {
            release_title: release_title.to_string(),
            series_title: String::new(),
            series_title_info: SeriesTitleInfo::default(),
            quality: QualityModel::new(tv::Unknown),
            season_number: 0,
            episode_numbers: Vec::new(),
            absolute_episode_numbers: Vec::new(),
            special_absolute_episode_numbers: Vec::new(),
            air_date: String::new(),
            languages: Vec::new(),
            full_season: false,
            is_partial_season: false,
            is_multi_season: false,
            is_season_extra: false,
            is_split_episode: false,
            is_mini_series: false,
            special: false,
            release_group: None,
            release_hash: String::new(),
            season_part: 0,
            release_tokens: String::new(),
            daily_part: None,
        }
    }

    /// A daily episode, found by its air date (`IsDaily`).
    pub fn is_daily(&self) -> bool {
        !self.air_date.trim().is_empty()
    }

    /// Numbered by absolute episode, as anime is (`IsAbsoluteNumbering`).
    pub fn is_absolute_numbering(&self) -> bool {
        !self.absolute_episode_numbers.is_empty()
    }

    /// Might be a special episode (`IsPossibleSpecialEpisode`).
    pub fn is_possible_special_episode(&self) -> bool {
        let no_title = self.series_title.trim().is_empty();
        (self.air_date.trim().is_empty() && no_title && (self.episode_numbers.is_empty() || self.season_number == 0))
            || (!no_title && self.special)
            || self.episode_numbers == [0]
    }

    /// Might be a scene special numbered as episode 0 of a season (`IsPossibleSceneSeasonSpecial`).
    pub fn is_possible_scene_season_special(&self) -> bool {
        self.season_number != 0 && self.episode_numbers == [0]
    }

    /// What kind of release this is (`ReleaseType`).
    pub fn release_type(&self) -> ReleaseType {
        if self.episode_numbers.len() > 1 || self.absolute_episode_numbers.len() > 1 {
            ReleaseType::MultiEpisode
        } else if self.episode_numbers.len() == 1 || self.absolute_episode_numbers.len() == 1 {
            ReleaseType::SingleEpisode
        } else if self.full_season {
            ReleaseType::SeasonPack
        } else {
            ReleaseType::Unknown
        }
    }
}

/// Compiles an upstream expression.
fn rx(pattern: &str, ignore_case: bool) -> ClrRegex {
    ClrRegex::new(pattern, ignore_case)
}

/// A table of expressions, compiled.
fn compile(table: &[(&str, bool)]) -> Vec<ClrRegex> {
    table.iter().map(|&(p, i)| rx(p, i)).collect()
}

static PRE_SUBSTITUTIONS: LazyLock<Vec<(ClrRegex, &str)>> =
    LazyLock::new(|| PRE_SUBSTITUTION.iter().map(|&(p, r)| (rx(p, false), r)).collect());
static REPORT_TITLES: LazyLock<Vec<ClrRegex>> = LazyLock::new(|| compile(REPORT_TITLE));
static SPECIAL_EPISODE_TITLES: LazyLock<Vec<ClrRegex>> = LazyLock::new(|| compile(SPECIAL_EPISODE_TITLE));
static REJECT_HASHED_RELEASES: LazyLock<Vec<ClrRegex>> = LazyLock::new(|| compile(REJECT_HASHED));

/// Declares a lazily compiled upstream expression.
macro_rules! regex {
    ($(#[$doc:meta])* $name:ident = $pattern:expr, $ignore_case:expr) => {
        $(#[$doc])*
        static $name: LazyLock<ClrRegex> = LazyLock::new(|| rx($pattern, $ignore_case));
    };
}

regex!(
    /// `SeasonFolderRegexes`: a season folder's name, not a release.
    SEASON_FOLDER_NAME = r"^(Season[ ._-]*\d+|Specials)$", false
);
regex!(
    /// `ReversedTitleRegex`: a name written backwards.
    REVERSED_TITLE = r"(?:^|[-._ ])(p027|p0801|\d{2,3}E\d{2}S)[-._ ]", false
);
regex!(
    /// `NormalizeRegex`: small words and punctuation dropped by [`clean_series_title`].
    NORMALIZE = r"((?:\b|_)(?<!^)([aà](?!$)|an|the|and|or|of)(?!$)(?:\b|_))|\W|_", true
);
regex!(
    /// `PercentRegex`
    PERCENT = r"(?<=\b\d+)%", false
);
regex!(
    /// `SimpleTitleRegex`: resolutions, codecs and similar that are taken out before the title is read.
    SIMPLE_TITLE = r"(?:(480|540|576|720|1080|2160)[ip]|[xh][\W_]?26[45]|DD\W?5\W1|[<>?*]|848x480|1280x720|1920x1080|3840x2160|4096x2160|(?<![a-f0-9])(8|10)(b(?![a-z0-9])|bit)|10-bit)\s*?", true
);
regex!(
    /// `WebsitePrefixRegex`
    WEBSITE_PREFIX = r"^(?:(?:\[|\()\s*)?(?:www\.)?[-a-z0-9-]{1,256}\.(?<!Naruto-Kun\.)(?:[a-z]{2,6}\.[a-z]{2,6}|xn--[a-z0-9-]{4,}|[a-z]{2,})\b(?:\s*(?:\]|\))|[ -]{2,})[ -]*", true
);
regex!(
    /// `WebsitePostfixRegex`
    WEBSITE_POSTFIX = r"(?:\[\s*)?(?:www\.)?[-a-z0-9-]{1,256}\.(?:xn--[a-z0-9-]{4,}|[a-z]{2,6})\b(?:\s*\])$", true
);
regex!(
    /// `SixDigitAirDateRegex`: a `yyMMdd` air date.
    SIX_DIGIT_AIR_DATE = r"(?<=[_.-])(?<airdate>(?<!\d)(?<airyear>[1-9]\d{1})(?<airmonth>[0-1][0-9])(?<airday>[0-3][0-9]))(?=[_.-])", true
);
regex!(
    /// `CleanReleaseGroupRegex`: posting-tool suffixes that aren't the group.
    CLEAN_RELEASE_GROUP = r"^(.*?[-._ ](S\d+E\d+)[-._ ])|(-(RP|1|NZBGeek|Obfuscated|Scrambled|sample|Pre|postbot|xpost|Rakuv[a-z0-9]*|WhiteRev|BUYMORE|AsRequested|AlternativeToRequested|GEROV|Z0iDS3N|Chamele0n|4P|4Planet|AlteZachen|RePACKPOST))+$", true
);
regex!(
    /// `CleanTorrentSuffixRegex`
    CLEAN_TORRENT_SUFFIX = r"\[(?:ettv|rartv|rarbg|cttv|publichd)\]$", true
);
regex!(
    /// `CleanQualityBracketsRegex`
    CLEAN_QUALITY_BRACKETS = r"\[[a-z0-9 ._-]+\]$", true
);
regex!(
    /// `ReleaseGroupRegex`
    RELEASE_GROUP = r"-(?<releasegroup>[a-z0-9]+(?<part2>-[a-z0-9]+)?(?!.+?(?:480p|576p|720p|1080p|2160p)))(?<!(?:WEB-DL|Blu-Ray|480p|576p|720p|1080p|2160p|DTS-HD|DTS-X|DTS-MA|DTS-ES|-ES|-EN|-CAT|[ ._]\d{4}-\d{2}|-\d{2})(?:\k<part2>)?)(?:\b|[-._ ]|$)|[-._ ]\[(?<releasegroup>[a-z0-9]+)\]$", true
);
regex!(
    /// `InvalidReleaseGroupRegex`
    INVALID_RELEASE_GROUP = r"^([se]\d+|[0-9a-f]{8})$", true
);
regex!(
    /// `AnimeReleaseGroupRegex`
    ANIME_RELEASE_GROUP = r"^(?:\[(?<subgroup>(?!\s).+?(?<!\s))\](?:_|-|\s|\.)?)", true
);
regex!(
    /// `ExceptionReleaseGroupRegexExact`: groups that don't use `-Group`.
    EXCEPTION_RELEASE_GROUP_EXACT = r"(?<releasegroup>(?:D\-Z0N3|Fight-BB|VARYG|E\.N\.D|KRaLiMaRKo|BluDragon|DarQ|KCRT|BEN[_. ]THE[_. ]MEN)\b)", true
);
regex!(
    /// `ExceptionReleaseGroupRegex`: groups whose releases end with `Group)` or `Group]`.
    EXCEPTION_RELEASE_GROUP = r"(?<=[._ \[])(?<releasegroup>(Silence|afm72|Panda|Ghost|MONOLITH|Tigole|Joy|ImE|UTR|t3nzin|Anime Time|Project Angel|Hakata Ramen|HONE|Vyndros|SEV|Garshasp|Kappa|Natty|RCVR|SAMPA|YOGI|r00t|EDGE2020|RZeroX)(?=\]|\)))", true
);
regex!(
    /// `YearInTitleRegex`
    YEAR_IN_TITLE = r"^(?<title>.+?)[-_. ]+?[\(\[]?(?<year>\d{4})[\]\)]?", true
);
regex!(
    /// `TitleComponentsRegex`: a name that gives several names.
    TITLE_COMPONENTS = r"^(?:(?<title>.+?) \((?<title>.+?)\)|(?<title>.+?) \| (?<title>.+?)|(?<title>.+?) AKA (?<title>.+?))$", true
);
regex!(
    /// `PartRegex`
    PART = r"\(\d+\)$", false
);
regex!(
    /// `PunctuationRegex`
    PUNCTUATION = r"[^\w\s]", false
);
regex!(
    /// `ArticleWordRegex`
    ARTICLE_WORD = r"^(a|an|the)\s", true
);
regex!(
    /// `DuplicateSpacesRegex`
    DUPLICATE_SPACES = r"\s{2,}", false
);
regex!(
    /// `SeasonFolderRegex`: the season of a season folder.
    SEASON_FOLDER = r"^(?:S|Season|Saison|Series|Stagione)[-_. ]*(?<season>(?<!\d+)\d{1,4}(?!\d+))(?:[_. ]+(?!\d+)|$)", true
);
regex!(
    /// `SimpleEpisodeNumberRegex`: a file named by its episode number alone.
    SIMPLE_EPISODE_NUMBER = r"^[ex]?(?<episode>(?<!\d+)\d{1,3}(?!\d+))(?:[ex-](?<episode>(?<!\d+)\d{1,3}(?!\d+)))?(?:[_. ](?!\d+)(?<remaining>.+)|$)", true
);
regex!(
    /// `RequestInfoRegex`: request tags in front of a name.
    REQUEST_INFO = r"^(?:\[.+?\])+", false
);
regex!(
    /// `MultiRegex`
    MULTI = r"[_. ](?<multi>multi)[_. ]", true
);

/// `Numbers`: number words a mini-series uses for its parts.
const NUMBERS: [&str; 10] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

/// `ValidateBeforeParsing`: whether a name is worth reading at all.
fn validate_before_parsing(title: &str) -> bool {
    let lower = title.to_lowercase();
    if lower.contains("password") && lower.contains("yenc") {
        return false;
    }
    if !title.chars().any(char::is_alphanumeric) {
        return false;
    }
    let without_extension = remove_file_extension(title);
    if REJECT_HASHED_RELEASES.iter().any(|r| r.is_match(&without_extension)) {
        return false;
    }
    !SEASON_FOLDER_NAME.is_match(&without_extension)
}

/// The name turned the right way round when it was written backwards (`ReversedTitleRegex`).
fn unreverse(title: &str) -> String {
    if !REVERSED_TITLE.is_match(title) {
        return title.to_string();
    }
    let without_extension = remove_file_extension(title);
    let reversed: String = without_extension.chars().rev().collect();
    format!("{reversed}{}", &title[without_extension.len()..])
}

/// `SimplifyTitle`: the name without site names, tracker tags or a reversed spelling.
pub fn simplify_title(title: &str) -> String {
    if !validate_before_parsing(title) {
        return title.to_string();
    }
    let title = unreverse(title);
    let simple = WEBSITE_PREFIX.replace_all(&title, "");
    let simple = WEBSITE_POSTFIX.replace_all(&simple, "");
    CLEAN_TORRENT_SUFFIX.replace_all(&simple, "")
}

/// `ParsePath`: reads a file path, using the folder names when the file name alone isn't enough.
pub fn parse_path(path: &str) -> Option<ParsedEpisodeInfo> {
    let p = std::path::Path::new(path);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or(path);
    let dir = p.parent().and_then(|d| d.file_name()).and_then(|n| n.to_str());
    let mut result = parse_title(name);

    // read the folder and the file separately, and put them together when both can be read
    if let (Some(episode_match), Some(dir)) = (SIMPLE_EPISODE_NUMBER.find(name), dir) {
        if result.as_ref().is_none_or(|r| r.is_mini_series || !r.absolute_episode_numbers.is_empty()) {
            if let Some(season_match) = SEASON_FOLDER.find(dir).filter(|m| m.ok("season")) {
                let episodes = episode_match.captures("episode");
                let first = parse_number(episodes.first()?.text).ok()?;
                let last = parse_number(episodes.last()?.text).ok()?;
                let mut path_title = format!("S{}E{first:02}", season_match.str("season"));
                if first != last {
                    path_title += &format!("-E{last:02}");
                }
                if let Some(remaining) = episode_match.group("remaining") {
                    path_title += &format!(" {}", remaining.text);
                }
                return parse_title(&path_title);
            }
        }
    }

    let dir = dir.unwrap_or("");
    if result.is_none() {
        let stem = p.file_stem().and_then(|n| n.to_str()).unwrap_or(name);
        if let Some(number) = dotnet_int(stem) {
            result = parse_title(dir).and_then(|mut r| {
                if r.absolute_episode_numbers.contains(&number) {
                    r.absolute_episode_numbers = vec![number];
                    Some(r)
                } else if r.episode_numbers.contains(&number) {
                    r.episode_numbers = vec![number];
                    Some(r)
                } else {
                    None
                }
            });
        }
    }
    if result.is_none() {
        result = parse_title(&format!("{dir} {name}"));
    }
    if result.is_none() {
        let extension = p.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
        result = parse_title(&format!("{dir}{extension}"));
    }
    result
}

/// `ParseTitle`: reads a release or file name; `None` when it can't be read.
pub fn parse_title(title: &str) -> Option<ParsedEpisodeInfo> {
    if !validate_before_parsing(title) {
        return None;
    }
    let title = unreverse(title);
    let mut release_title = remove_file_extension(&title).replace('【', "[").replace('】', "]");
    for (regex, replacement) in PRE_SUBSTITUTIONS.iter() {
        if regex.is_match(&release_title) {
            release_title = regex.replace_all(&release_title, replacement);
        }
    }

    let simple = SIMPLE_TITLE.replace_all(&release_title, "");
    let simple = WEBSITE_PREFIX.replace_all(&simple, "");
    let simple = WEBSITE_POSTFIX.replace_all(&simple, "");
    let simple = CLEAN_TORRENT_SUFFIX.replace_all(&simple, "");
    let mut simple = CLEAN_QUALITY_BRACKETS.replace_all_with(&simple, |m| {
        let text = m.whole().text;
        if quality_tv::parse_quality_name(text).quality != tv::Unknown {
            String::new()
        } else {
            text.to_string()
        }
    });

    if let Some(m) = SIX_DIGIT_AIR_DATE.find(&simple) {
        let (year, month, day) = (m.str("airyear"), m.str("airmonth"), m.str("airday"));
        if month != "00" || day != "00" {
            let fixed = format!("20{year}.{month}.{day}");
            let air_date = m.str("airdate").to_string();
            simple = simple.replace(&air_date, &fixed);
        }
    }

    for regex in REPORT_TITLES.iter() {
        let matches = regex.find_all(&simple);
        if matches.is_empty() {
            continue;
        }
        let mut result = match parse_match_collection(&matches, &simple, &release_title) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            // an air date that can't be right, or a number that can't be read, ends the search (upstream throws)
            Err(Stop) => return None,
        };
        if result.full_season && has_i(&result.release_tokens, "Special") {
            result.full_season = false;
            result.special = true;
        }
        result.languages = language_tv::parse_languages(&result.release_tokens);
        result.quality = quality_tv::parse_quality(&title);
        result.release_group = parse_release_group(&release_title);
        if let Some(sub) = matches[0].group("subgroup").filter(|s| !s.text.trim().is_empty()) {
            result.release_group = Some(sub.text.to_string());
        }
        result.release_hash = release_hash(&matches[0]);
        return Some(result);
    }
    None
}

/// `string.ContainsIgnoreCase`.
fn has_i(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// `ParseSeriesName`: the series name of a release, or the cleaned name when it can't be read.
pub fn parse_series_name(title: &str) -> String {
    match parse_title(title) {
        Some(r) => r.series_title,
        None => clean_series_title(title),
    }
}

/// `CleanSeriesTitle`: the name in lower case without punctuation, small words or accents, for comparing.
pub fn clean_series_title(title: &str) -> String {
    if title.trim().is_empty() || dotnet_long(title).is_some() {
        return title.to_string();
    }
    let title = PERCENT.replace_all(title, "percent");
    remove_accent(&NORMALIZE.replace_all(&title, "").to_lowercase())
}

/// `NormalizeEpisodeTitle`: an episode title in lower case without punctuation, for comparing.
pub fn normalize_episode_title(title: &str) -> String {
    let mut title = title.to_string();
    if let Some(m) = SPECIAL_EPISODE_TITLES.iter().find_map(|r| r.find(&title)) {
        title = m.str("episodetitle").to_string();
    }
    let title = PART.replace_all(&title, "");
    let title = PUNCTUATION.replace_all(&title, " ");
    let title = DUPLICATE_SPACES.replace_all(&title, " ");
    title.trim().to_lowercase()
}

/// `NormalizeTitle`: a title without punctuation or a leading article, in lower case.
pub fn normalize_title(title: &str) -> String {
    let title = PUNCTUATION.replace_all(title, "");
    let title = ARTICLE_WORD.replace_all(&title, "");
    let title = DUPLICATE_SPACES.replace_all(&title, " ");
    title.trim().to_lowercase()
}

/// `ParseReleaseGroup`: the group that made a release; `None` when there is none.
pub fn parse_release_group(title: &str) -> Option<String> {
    let mut title = remove_file_extension(title.trim());
    for (regex, replacement) in PRE_SUBSTITUTIONS.iter() {
        if regex.is_match(&title) {
            title = regex.replace_all(&title, replacement);
            break;
        }
    }
    let title = WEBSITE_PREFIX.replace_all(&title, "");
    let title = CLEAN_TORRENT_SUFFIX.replace_all(&title, "");
    if let Some(m) = ANIME_RELEASE_GROUP.find(&title) {
        return Some(m.str("subgroup").to_string());
    }
    let title = CLEAN_RELEASE_GROUP.replace_all(&title, "");
    if let Some(m) = EXCEPTION_RELEASE_GROUP.find_all(&title).last() {
        return Some(m.str("releasegroup").to_string());
    }
    if let Some(m) = EXCEPTION_RELEASE_GROUP_EXACT.find_all(&title).last() {
        return Some(m.str("releasegroup").to_string());
    }
    let matches = RELEASE_GROUP.find_all(&title);
    let group = matches.last()?.str("releasegroup");
    if dotnet_int(group).is_some() || INVALID_RELEASE_GROUP.is_match(group) {
        return None;
    }
    Some(group.to_string())
}

/// `HasMultipleLanguages`: whether the name says `multi`.
pub fn has_multiple_languages(title: &str) -> bool {
    MULTI.is_match(title)
}

/// `SceneChecker.IsSceneTitle`: whether a name looks like a scene release name. Prefers saying no when unsure.
pub fn is_scene_title(title: &str) -> bool {
    if !title.contains('.') || title.contains(' ') {
        return false;
    }
    parse_title(title).is_some_and(|p| p.release_group.is_some() && p.quality.quality != tv::Unknown && !p.series_title.trim().is_empty())
}

/// `GetSeriesTitleInfo`
fn series_title_info(title: &str, first: &ClrMatch<'_>) -> SeriesTitleInfo {
    let mut info = SeriesTitleInfo { title: title.to_string(), ..SeriesTitleInfo::default() };
    match YEAR_IN_TITLE.find(title) {
        None => info.title_without_year = title.to_string(),
        Some(m) => {
            info.title_without_year = m.str("title").to_string();
            info.year = dotnet_int(m.str("year")).unwrap_or(0);
        }
    }
    if let Some(m) = TITLE_COMPONENTS.find(&info.title_without_year) {
        info.all_titles = m.captures("title").iter().map(|c| c.text.to_string()).collect();
    } else {
        let titles = first.captures("title");
        if titles.len() > 1 {
            info.all_titles = titles.iter().map(|c| c.text.replace(['.', '_'], " ")).collect();
        }
    }
    info
}

/// Where a capture ends in the text, in UTF-16 units (`EndIndex`).
fn end_index(text: &str, end: usize) -> usize {
    utf16_len(&text[..end])
}

/// The reason reading stopped where upstream throws.
struct Stop;

/// `ParseMatchCollection`: the numbers of a matched name. `Ok(None)` lets the next expression try; `Err` ends the
/// search.
fn parse_match_collection(matches: &[ClrMatch<'_>], simple: &str, release_title: &str) -> Result<Option<ParsedEpisodeInfo>, Stop> {
    let first = &matches[0];
    let series_name = first.str("title").replace(['.', '_'], " ");
    let series_name = REQUEST_INFO.replace_all(&series_name, "").trim_matches(' ').to_string();
    let air_year = dotnet_int(first.str("airyear")).unwrap_or(0);
    let mut last_index = first.group("title").map_or(0, |t| end_index(simple, t.end));

    let mut result;
    if air_year < 1900 {
        result = ParsedEpisodeInfo::new(release_title);
        for m in matches {
            let episodes = m.captures("episode");
            let absolutes = m.captures("absoluteepisode");

            // no episodes at all is fine: that is read as a whole season
            if let (Some(a), Some(b)) = (episodes.first(), episodes.last()) {
                let (lo, hi) = (parse_number(a.text)?, parse_number(b.text)?);
                if lo > hi {
                    return Ok(None);
                }
                result.episode_numbers = (lo..=hi).collect();
                last_index = last_index.max(end_index(simple, b.end));
                if m.ok("special") {
                    result.special = true;
                }
                if m.ok("splitepisode") {
                    result.is_split_episode = true;
                }
            }

            if let (Some(a), Some(b)) = (absolutes.first(), absolutes.last()) {
                let (lo, hi) = (parse_decimal(a.text)?, parse_decimal(b.text)?);
                if lo > hi {
                    return Ok(None);
                }
                if lo.fract() != 0.0 || hi.fract() != 0.0 {
                    if absolutes.len() != 1 {
                        // several numbers are not allowed for specials
                        return Ok(None);
                    }
                    result.special_absolute_episode_numbers = vec![lo];
                    result.special = true;
                    last_index = last_index.max(end_index(simple, a.end));
                } else {
                    let (lo, hi) = (to_int(lo)?, to_int(hi)?);
                    result.absolute_episode_numbers = (lo..=hi).collect();
                    if m.ok("special") {
                        result.special = true;
                    }
                    last_index = last_index.max(end_index(simple, b.end));
                }
            }

            if episodes.is_empty() && absolutes.is_empty() {
                // extras and subtitle packs are marked so they can be left out
                if !first.str("extras").trim().is_empty() {
                    result.is_season_extra = true;
                }
                // a partial season pack has a part number, which tells it from a whole season
                let season_part = first.str("seasonpart");
                if !season_part.trim().is_empty() {
                    result.season_part = convert_to_int(season_part)?;
                    result.is_partial_season = true;
                } else {
                    result.full_season = true;
                }
            }

            if episodes.len() == 2 && first.ok("episodecount") && episodes[1].text == first.str("episodecount") {
                result.episode_numbers = Vec::new();
                result.full_season = true;
            }
        }

        let mut seasons = Vec::new();
        for capture in first.captures("season") {
            if let Some(season) = dotnet_int(capture.text) {
                seasons.push(season);
                last_index = last_index.max(end_index(simple, capture.end));
            }
        }
        // more than one season is marked so it can be turned down later
        let mut distinct = seasons.clone();
        distinct.sort_unstable();
        distinct.dedup();
        if distinct.len() > 1 {
            result.is_multi_season = true;
        }
        if let Some(&season) = seasons.first() {
            result.season_number = season;
        } else if result.absolute_episode_numbers.is_empty() && !result.episode_numbers.is_empty() {
            // no season and not numbered by absolute episode: a mini-series, season 1
            result.season_number = 1;
            result.is_mini_series = true;
        }
    } else {
        // a daily show
        let (month, day);
        if first.ok("ambiguousairmonth") && first.ok("ambiguousairday") {
            let ambiguous_month = convert_to_int(first.str("ambiguousairmonth"))?;
            let ambiguous_day = convert_to_int(first.str("ambiguousairday"))?;
            if ambiguous_day <= 12 && ambiguous_month <= 12 {
                // month and day can't be told apart
                return Err(Stop);
            }
            (month, day) = (ambiguous_month, ambiguous_day);
        } else {
            let (m, d) = (convert_to_int(first.str("airmonth"))?, convert_to_int(first.str("airday"))?);
            // a month past 12 means the scene swapped day and month
            (month, day) = if m > 12 { (d, m) } else { (m, d) };
        }
        let air_date = u32::try_from(month)
            .ok()
            .zip(u32::try_from(day).ok())
            .and_then(|(m, d)| NaiveDate::from_ymd_opt(air_year, m, d))
            .ok_or(Stop)?;
        // a date in the future is most likely a misreading
        let tomorrow = chrono::Local::now().date_naive() + chrono::Days::new(1);
        if air_date > tomorrow {
            return Err(Stop);
        }
        // before 1970 only with the year in the title (Plex DVR names)
        if air_date.year() < 1970 && first.str("titleyear").trim().is_empty() {
            return Err(Stop);
        }
        for name in ["airyear", "airmonth", "airday"] {
            if let Some(g) = first.group(name) {
                last_index = last_index.max(end_index(simple, g.end));
            }
        }
        result = ParsedEpisodeInfo::new(release_title);
        result.air_date = air_date.format("%Y-%m-%d").to_string();
        if let Some(part) = first.group("part") {
            result.daily_part = Some(convert_to_int(part.text)?);
        }
    }

    // upstream counts the place in the simple title but cuts the release title there
    result.release_tokens = match byte_at_utf16(release_title, last_index) {
        Some(at) if last_index < utf16_len(release_title) => release_title[at..].to_string(),
        _ => release_title.to_string(),
    };
    result.series_title = series_name;
    result.series_title_info = series_title_info(&result.series_title, first);
    Ok(Some(result))
}

/// `GetReleaseHash`: the hash of an anime release, without its brackets.
fn release_hash(first: &ClrMatch<'_>) -> String {
    match first.group("hash") {
        Some(h) => {
            let value = h.text.trim_matches(['[', ']', '(', ')']);
            if value == "1280x720" {
                String::new()
            } else {
                value.to_string()
            }
        }
        None => String::new(),
    }
}

/// `Convert.ToInt32` of a string: an error where .NET throws.
fn convert_to_int(s: &str) -> Result<i32, Stop> {
    dotnet_int(s).ok_or(Stop)
}

/// A whole decimal as an `i32`.
fn to_int(n: f64) -> Result<i32, Stop> {
    if n.abs() < f64::from(i32::MAX) {
        Ok(n as i32)
    } else {
        Err(Stop)
    }
}

/// The value of a digit of any script (`char.GetNumericValue` for decimal digits), found from the run of digits it
/// sits in: Unicode places each script's digits zero to nine in a row.
fn digit_value(c: char) -> Option<u32> {
    if let Some(d) = c.to_digit(10) {
        return Some(d);
    }
    static DIGIT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\d$").expect("valid"));
    let digit = |c: char| DIGIT.is_match(c.encode_utf8(&mut [0; 4]));
    if !digit(c) {
        return None;
    }
    let mut start = u32::from(c);
    while start > 0 && char::from_u32(start - 1).is_some_and(digit) {
        start -= 1;
    }
    Some((u32::from(c) - start) % 10)
}

/// `ConvertToNumerals` after `Normalize(FormKC)`: digits of other scripts written as 0–9.
fn to_numerals(value: &str) -> String {
    value.nfkc().map(|c| digit_value(c).and_then(|d| char::from_digit(d, 10)).unwrap_or(c)).collect()
}

/// `ParseNumber`: an episode number, in digits of any script or as a word (`one`).
fn parse_number(value: &str) -> Result<i32, Stop> {
    if let Some(n) = dotnet_int(&to_numerals(value)) {
        return Ok(n);
    }
    let lower = value.to_lowercase();
    NUMBERS.iter().position(|w| *w == lower).map(|i| i as i32).ok_or(Stop)
}

/// `ParseDecimal`: an absolute episode number that may have a fraction (`12.5`).
fn parse_decimal(value: &str) -> Result<f64, Stop> {
    let normalized = to_numerals(value);
    let t = normalized.trim();
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '+' | '-' | 'e' | 'E')) {
        return Err(Stop);
    }
    t.parse().map_err(|_| Stop)
}

/// `NormalizeImdbId`: an IMDb id written the usual way (`tt0123456`), or `None` when it isn't one.
pub fn normalize_imdb_id(imdb_id: &str) -> Option<String> {
    super::movie::normalize_imdb_id(imdb_id)
}

/// `PreSubstitutionRegex`: rewrites some anime and Korean names into a shape the title expressions read
/// (expression, replacement; case matters).
const PRE_SUBSTITUTION: &[(&str, &str)] = &[
    (r#"\.E(\d{2,4})\.\d{6}\.(.*-NEXT)$"#, r#".S01E$1.$2"#),
    (
        r#"^\[(?:(?<subgroup>[^\]]+?)(?:[\u4E00-\u9FCC]+)?)\]\[(?<title>[^\]]+?)(?:\s(?<chinesetitle>[\u4E00-\u9FCC][^\]]*?))\]\[(?:(?:[\u4E00-\u9FCC]+?)?(?<episode>\d{1,4})(?:[\u4E00-\u9FCC]+?)?)\]"#,
        r#"[${subgroup}] ${title} - ${episode} - "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]*?(?:LoliHouse|ZERO|Lilith-Raws|Skymoon-Raws|orion origin)[^\]]*?)\](?<title>[^\[\]]+?)(?: - (?<episode>[0-9-]+)\s*|\[第?(?<episode>[0-9]+(?:-[0-9]+)?)话?(?:END|完)?\])\["#,
        r#"[${subgroup}][${title}][${episode}]["#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s?★[^\[ -]+\s?)?\[?(?:(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:\]\[|\s*[_/·]\s*)){0,2}(?<title>[^\[\]]+?)(?:\s(?:S?(?<!\d+)((0)(?<season>\d)|(?<season>[1-9]\d))(?!\d+)))\]?(?:\[\d{4}\])?\[第?(?<episode>[0-9]+(?:-[0-9]+)?)(?:话|集)?(?: ?END|完| ?Fin)?\]"#,
        r#"[${subgroup}] ${title} S${season} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\]\[(?<chinesetitle>(?<![^a-zA-Z0-9])[^a-zA-Z0-9]+)(?<title>[^\]]+?)\](?:\[\d{4}\])?\[第?(?<episode>[0-9]+(?:-[0-9]+)?)(?:话|集)?(?: ?END|完| ?Fin)?\]"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s?★[^\[ -]+\s?)?\[?(?:(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:\]\[|\s*[_/·]\s*)){0,2}(?<title>[^\]]+?)\]?(?:\[\d{4}\])?\[第?(?<episode>[0-9]{1,4}(?:-[0-9]{1,4})?)(?:话|集)?(?: ?END|完| ?Fin)?\]"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s)(?:(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:\s/\s))(?<title>[^\[\]]+?)(?:\s(?:S?(?<!\d+)((0)(?<season>\d)|(?<season>[1-9]\d))(?!\d+)))(?:[- ]+)(?<episode>[0-9]+(?:-[0-9]+)?)话?(?:END|完)?"#,
        r#"[${subgroup}] ${title} S${season} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s)(?:(?<title>[^\]]+?)(?:\s/\s))(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:[- ]+)(?<episode>[0-9]+(?:-[0-9]+)?(?![a-z]))话?(?:END|完)?"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s)(?:(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:\s/\s))(?<title>[^\]]+?)(?:[- ]+)(?<episode>[0-9]+(?:-[0-9]+)?(?![a-z]))话?(?:END|完)?"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:(?<chinesubgroup>\[(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*\])+)\[(?<title>[^\]]+?)\](?<junk>\[[^\]]+\])*\[(?<episode>[0-9]+(?:-[0-9]+)?)( END| Fin)?\]"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (
        r#"^\[(?<subgroup>[^\]]+)\](?:\s)(?:(?<chinesetitle>(?=[^\]]*?[\u4E00-\u9FCC])[^\]]*?)(?:\s\|\s))(?<title>[^\]]+?)(?:[- ]+)(?<episode>[0-9]+(?:-[0-9]+)?(?![a-z]))话?(?:END|完)?"#,
        r#"[${subgroup}] ${title} - ${episode} "#,
    ),
    (r#"^(?<title>.+?(?=[ ._-]\()).+?\((?<year>\d{4})\/(?<info>S[^\/]+)"#, r#"${title} (${year}) - ${info} "#),
];

/// `ReportTitleRegex`: the title expressions, tried in order (expression, ignore case).
const REPORT_TITLE: &[(&str, bool)] = &[
    (
        r#"^^(?<title>.+?\((?<titleyear>\d{4})\))[-_. ]+(?<airyear>19[4-9]\d|20\d\d)(?<sep>[-_]?)(?<airmonth>0\d|1[0-2])\k<sep>(?<airday>[0-2]\d|3[01])[-_. ]\d{2}[-_. ]\d{2}[-_. ]\d{2}"#,
        true,
    ),
    (r#"^(?<airyear>19[6-9]\d|20\d\d)(?<sep>[-_]?)(?<airmonth>0\d|1[0-2])\k<sep>(?<airday>[0-2]\d|3[01])(?!\d)"#, true),
    (r#"^(?:\W*S(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:e{1,2}(?<episode>\d{1,3}(?!\d+)))+){2,}"#, true),
    (r#"^(?:\W*(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:x{1,2}(?<episode>\d{1,3}(?!\d+)))+){2,}"#, true),
    (r#"^(?:S?(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:[-_]|[ex]){1,2}(?<episode>\d{2,3}(?!\d+))){2,})"#, true),
    (
        r#"^(?<title>.+?)(?:S?(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:[-_ ]?[ex])(?<episode>\d{2,3}(?!\d+))(?<splitepisode>[a-d])(?:[ _.])))"#,
        true,
    ),
    (r#"^(?:S?(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:[-_ ]?[ex])(?<episode>\d{2,3}(?!\d+))))"#, true),
    (
        r#"^(?:\[(?<subgroup>.+?)\](?:_|-|\s|\.)?)(?<title>.+?)[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+))(?:[-_. ])+\((?:S(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:[ex]|\W[ex]){1,2}(?<episode>\d{2}(?!\d+))))(?:v\d+)?(?:\)(?!\d+)).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\](?:_|-|\s|\.)?)(?<title>.+?)[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+))(?:[-_. ](?<![()\[!]))+(?:S(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:[ex]|\W[ex]){1,2}(?<episode>\d{2}(?!\d+))))(?:v\d+)?(?:[_. ](?!\d+)).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\](?:_|-|\s|\.)?)(?<title>.+?)(?:[-_\W](?<![()\[!]))+(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:[ex]|\W[ex]){1,2}(?<episode>\d{2}(?!\d+)))+)(?:v\d+)?(?:[_. ](?!\d+)).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>.+?)[-_. ]+?(?:Episode)(?:[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+)))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\](?:_|-|\s|\.)?)(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+(?<absoluteepisode>\d{2,3}(\.\d{1,2})?))+(?:_|-|\s|\.)+(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:\-|[ex]|\W[ex]){1,2}(?<episode>\d{2}(?!\d+)))+).*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.)"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\](?:_|-|\s|\.)?)(?<title>.+?)(?:[-_\W](?<![()\[!]))+(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:\-|[ex]|\W[ex]){1,2}(?<episode>\d{2}(?!\d+)))+)(?:(?:_|-|\s|\.)+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+|\-[a-z])))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>.+?[^-]+?)(?:(?<![-_. ]|\b[0]\d+) - )[-_. ]?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+))\s?~\s?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+))(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>[^-]+?)[_. ]+?\(Season[_. ](?<season>\d+)\)[-_. ]+?(?:[-_. ]?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+)))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>[^]]+?)(?:[-_. ]{3}?(?<absoluteepisode>\d{2}(\.\d{1,2})?(?!-?\d+|-[a-z]+)))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>[^-]+?)(?:(?<![-_. ]|\b[0]\d+) - )(?:[-_. ]?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+)))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>.+?)[-_. ]+(?:S(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?![ex]?\d+))).+?(?:$|\.mkv)"#, true),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>[^-]+?)(?:(?<![-_. ]|\b[0]\d+)[_ ]+)(?:[-_. ]?(?<absoluteepisode>\d{3}(\.\d{1,2})?(?!\d+|-[a-z]+)))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>.+?)(?:(?<!\b[0]\d+))(?:[. ]-[. ](?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+|[-])))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>.+?)(?:(?<!\b[0]\d+))(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+|[-]))[. ]-[. ](?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+|[-]))(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^\[(?<subgroup>.+?)\][-_. ]?(?<title>.+?)[-_. ]+\(?(?:[-_. ]?#?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+|-[a-z]+)))+\)?(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])?(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:e|[-_. ]e){1,2}(?<episode>\d{1,3}(?!\d+)))+){2,}"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:x{1,2}(?<episode>\d{1,3}(?!\d+)))+){2,}"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:[ex]|\W[ex]|_){1,2}(?<episode>\d{2,3}(?!\d+|(?:[ex]|\W[ex]|_|-){1,2}\d+))(?<special>\.5[ .]\[SP\]))"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:[ex]|\W[ex]|_){1,2}(?<episode>\d{2,3}(?!\d+|(?:[ex]|\W[ex]|_|-){1,2}\d+))).+?(?:\[.+?\])(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:[-_\W](?<![()\[!]))+(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:[ex]|\W[ex]|-){1,2}(?<episode>(?<!\d+)\d{2}(?!\d+)))+)[-_. (]+?(?:[-_. ]?(?<absoluteepisode>(?<!\d+)\d{3}(\.\d{1,2})?(?!\d+|[pi])))+.+?\[(?<subgroup>.+?)\](?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:[ex]|\W[ex]|_){1,2}(?<episode>\d{2,3}(?!\d+))(?:(?:\-|[ex]|\W[ex]|_){1,2}(?<episode>\d{2,3}(?!\d+)))+).+?(?:\[.+?\])(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ]Episode(?:[-_. ]+(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+)))+(?:.+?)\[(?<subgroup>.+?)\].*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:_|-|\s|\.)+(?<absoluteepisode>\d{3}(\.\d{1,2})(?!\d+)))+(?:.+?)\[(?<subgroup>.+?)\].*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (r#"^(?<title>.+?)[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2}(?!\d+))[-_. ](\(\d{4}\))[-_. ]\[(?<subgroup>.+?)\]"#, true),
    (
        r#"^(?<title>[^-]+?)(?:(?<![-_. ]|\b[0]\d+) - )(?:[-_. ]?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+)))+(?:[-_. ]+(?<special>special|ova|ovd))?.*?(?<hash>[(\[]\w{8}[)\]])(?:$|\.mkv)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:_|-|\s|\.)+(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+)))+(?:[-_. ]+(?<special>special|ova|ovd))?[-_. ]+.*?(?<hash>[(\[]\w{8}[)\]])$"#,
        true,
    ),
    (
        r#"^(?<title>.+?)?\W*(?<airdate>\d{4}\W+[0-1][0-9]\W+[0-3][0-9])(?!\W+[0-3][0-9])[-_. ](?:s?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+)))(?:[ex](?<episode>(?<!\d+)(?:\d{1,3})(?!\d+)))"#,
        true,
    ),
    (
        r#"^(?<title>.+?)?\W*(?<airyear>\d{4})\W+(?<airmonth>[0-1][0-9])\W+(?<airday>[0-3][0-9])(?!\W+[0-3][0-9]).+?(?:s?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+)))(?:[ex](?<episode>(?<!\d+)(?:\d{1,3})(?!\d+)))"#,
        true,
    ),
    (
        r#"^(?<title>.+?)?[-_. ](?:e\d{2,3}(?!\d+))[-_. ](?<airyear>\d{4})\W+(?<airmonth>[0-1][0-9])\W+(?<airday>[0-3][0-9])(?!\W+[0-3][0-9]).+?(?:\[[a-z]+\])"#,
        true,
    ),
    (
        r#"^((?<title>.*?)[ ._]\/[ ._])+\(?S(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:\W|_)?E?[ ._]?(?<episode>(?<!\d+)\d{1,2}(?!\d+))(?:-(?<episode>(?<!\d+)\d{1,2}(?!\d+)))?(?:[ ._]of[ ._](?<episodecount>\d{1,2}))?\)?[ ._][\(\[]"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:[-_\W](?<![()\[!]))+S(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))E(?<episode>\d{2,3}(?!\d+))(?:-(?<episode>\d{2,3}(?!\d+)))+(?:[-_. ]|$)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:[-_\W](?<![()\[!]))+S(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))E(?<episode>\d{1,2}(?!\d+))(?:-(?<episode>\d{1,2}(?!\d+)))+(?:[-_. ]|$)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:[ex]|\W[ex]){1,2}(?<episode>\d{2,3}(?!\d+))(?:(?:\-|[ex]|\W[ex]|_){1,2}(?<episode>\d{2,3}(?!\d+)))*)(?:[-_. ]|$)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+S(?<season>(?<!\d+)(?:\d{4})(?!\d+))(?:e|\We|_){1,2}(?<episode>\d{2,4}(?!\d+))(?:(?:\-|e|\We|_){1,2}(?<episode>\d{2,3}(?!\d+)))*)\W?(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+(?<season>(?<!\d+)(?:\d{4})(?!\d+))(?:x|\Wx){1,2}(?<episode>\d{2,4}(?!\d+))(?:(?:\-|x|\Wx|_){1,2}(?<episode>\d{2,3}(?!\d+)))*)\W?(?!\\)"#,
        true,
    ),
    (r#"^(?<title>.+?)(?:[-_\W](?<![()\[!]))+S(?<season>(?<!\d+)(?:\d{2})(?!\d+))(?:\.)(?<episode>\d{2,3}(?!\d+))(?:[-_. ]|$)"#, true),
    (
        r#"^(?<title>.+?)(Complete Series)?[-_. ]+(?:S|(?:Season|Saison|Series|Stagione)[_. ])(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:[-_. ]{1}|[-_. ]{3})(?:S|(?:Season|Saison|Series|Stagione)[_. ])?(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:\W+S(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))\W+(?:(?:(?:Part|Vol)\W?|(?<!\d+\W+)e|p)(?<seasonpart>\d{1,2}(?!\d+)))+)"#,
        true,
    ),
    (r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>.+?)[-_. ]+?(?<absoluteepisode>\d{4}(\.\d{1,2})?(?!\d+))"#, true),
    (r#"^(?<title>.+?)[-_. ]+(?<absoluteepisode>(?<!\d+)\d{4}(?!\d+))[-_. ]\[(?<subgroup>.+?)\]"#, true),
    (r#"^(?<title>.+?\d{4})(?:\W+(?:(?:Part\W?|e)(?<episode>\d{1,2}(?!\d+)))+)"#, true),
    (r#"^(?<title>.+?)(?:[-._ ][e])(?<episode>\d{2,3}(?!\d+))(?:(?:\-?[e])(?<episode>\d{2,3}(?!\d+)))+"#, true),
    (
        r#"^(?<title>.+?)?\W*(?<airyear>\d{4})[-_. ]+(?<airmonth>[0-1][0-9])[-_. ]+(?<airday>[0-3][0-9])(?![-_. ]+[0-3][0-9])[-_. ]+Part[-_. ]?(?<part>[1-9])"#,
        true,
    ),
    (r#"^(?<title>.+?)(?:\W+(?:(?:(?<!\()Part\W?|(?<!\d+\W+)e)(?<episode>\d{1,2}(?!\d+|\))))+)"#, true),
    (r#"^(?<title>.+?)(?:\W+(?:Part[-._ ](?<episode>One|Two|Three|Four|Five|Six|Seven|Eight|Nine)(?>[-._ ])))"#, true),
    (r#"^(?<title>.+?)(?:\W+(?:(?<episode>(?<!\d+)\d{1,2}(?!\d+))of\d+)+)"#, true),
    (
        r#"(?:.*(?:\"|^))(?<title>.*?)(?:[-_\W](?<![()\[]))+(?:\W?Season\W?)(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:\W|_)+(?:Episode\W)(?:[-_. ]?(?<episode>(?<!\d+)\d{1,2}(?!\d+)))+"#,
        true,
    ),
    (r#"(?:.*(?:^))(?<title>.*?)[-._ ]+\[S(?<season>(?<!\d+)\d{2}(?!\d+))(?:[E-]{1,2}(?<episode>(?<!\d+)\d{2}(?!\d+)))+\]"#, true),
    (r#"(?:.*(?:^))(?<title>.*?)[-._ ]+\(S(?<season>(?<!\d+)\d{2}(?!\d+))(?:[E-]{1,2}(?<episode>(?<!\d+)\d{2}(?!\d+)))+\)"#, true),
    (r#"(?:.*(?:^))(?<title>.*?)S(?<season>(?<!\d+)\d{2}(?!\d+))(?:E(?<episode>(?<!\d+)\d{2}(?!\d+)))+"#, true),
    (r#"^(?<title>.+?)[-_. ]S(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:[-_. ]?[ex]?(?<episode>(?<!\d+)\d{1,2}(?!\d+)))+"#, true),
    (
        r#"^(?<title>.*?)[ ._]\(S(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:\W|_)?E?[ ._]?(?<episode>(?<!\d+)\d{1,2}(?!\d+))(?:-(?<episode>(?<!\d+)\d{1,2}(?!\d+)))?\)(?:[ ._]\/[ ._])(?<title>.*?)[ ._]\("#,
        true,
    ),
    (r#"(?:.*(?:\"|^))(?<title>.*?)(?:\W?|_)S(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:\W|_)?Ep?[ ._]?(?<episode>(?<!\d+)\d{1,2}(?!\d+))"#, true),
    (r#"(?:.*(?:\"|^))(?<title>.*?)(?:\W?|_)S(?<season>(?<!\d+)\d{3}(?!\d+))(?:\W|_)?E(?<episode>(?<!\d+)\d{1,2}(?!\d+))"#, true),
    (
        r#"^(?:(?<title>.+?)(?:_|-|\s|\.)+)(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+)))(?:(?:\-|[ex]|\W[ex]|_){1,2}(?<episode>(?<!\d+)\d{5}(?!\d+)))"#,
        true,
    ),
    (
        r#"^(?:(?<title>.+?)(?:_|-|\s|\.)+)(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+)))(?:(?:[-_. ]{1,3}ep){1,2}(?<episode>(?<!\d+)\d{5}(?!\d+)))+"#,
        true,
    ),
    (r#"^(?<title>.+?)(?:_|-|\s|\.)+S(?<season>\d{2}(?!\d+))(\W-\W)E(?<episode>(?<!\d+)\d{2}(?!\d+))(?!\\)"#, true),
    (
        r#"^(?<title>.+?)?(?:[-_\W](?<![()\[!]))+\[(?:s)?(?<season>(?<!\d+)\d{1,2})(?:(?:[ex])(?<episode>\d{2}))(?:(?:[-ex]){1,2}(?<episode>\d{2}))*\].+?(?:\.|$)"#,
        true,
    ),
    (r#"^(?<title>.+?S\d{1,2})[-_. ]{3,}(?:EP)?(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+|[-]))"#, true),
    (r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)[-_. ]+?(?:Episode[-_. ]+?)(?<absoluteepisode>\d{1}(\.\d{1,2})?(?!\d+))"#, true),
    (r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>.+?)[-_. ]+?\[(?<absoluteepisode>\d{2,3}(\.\d{1,2})?(?!\d+))\]"#, true),
    (
        r#"^(?<airyear>\d{2})(?<airmonth>[0-1][0-9])(?<airday>[0-3][0-9])(?![-_. ]+[0-3][0-9])[-_. ](?<title>.+?)[-_. ](?:Season[-_. ]?(?<season>\d{1,2})[-_. ])?(?:ep|#)(?<episode>\d{2,3})"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ]+?(?:S|Season|Saison|Series|Stagione)[-_. ]?(?<season>\d{1,2}(?=[-_. ]\d{4}[-_. ]+))(?<extras>EXTRAS|SUBPACK)?(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ]+?(?:S|Season|Saison|Series|Stagione)[-_. ]?(?<season>\d{1,2}(?![-_. ]?\d+))(?:[-_. ]|$)+(?<extras>EXTRAS|SUBPACK)?(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ]+?(?:S|Season|Saison|Series|Stagione)[-_. ]?(?<season>\d{4}(?![-_. ]?\d+))(\W+|_|$)(?<extras>EXTRAS|SUBPACK)?(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)(?:(?:[-_. ]+?Temporada.+?|\[.+?\])\[Cap)(?:[-_. ]+(?<season>(?<!\d+)\d{1,2})(?<episode>(?<!e|x)(?:[1-9][0-9]|[0][1-9])))+(?:\])"#,
        true,
    ),
    (r#"^(?<title>.+?)?(?:(?:[_.-](?<![()\[!]))+(?<season>(?<!\d+)[1-9])(?<episode>[1-9][0-9]|[0][1-9])(?![a-z]|\d+))+(?:[_.]|$)"#, true),
    (r#"^(?:S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:\-|[ex]|\W[ex]|_){1,2}(?<episode>\d{4}(?!\d+|i|p)))+)(\W+|_|$)(?!\\)"#, true),
    (
        r#"^(?<title>.+?)(?:(?:[-_\W](?<![()\[!]|\d{1,2}-))+S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:\-|[ex]|\W[ex]|_){1,2}(?<episode>\d{4}(?!\d+|i|p)))+)\W?(?!\\)"#,
        true,
    ),
    (r#"^(?<title>.+?)?\W*(?<airyear>\d{4})[-_. ]+(?<airmonth>[0-1][0-9])[-_. ]+(?<airday>[0-3][0-9])(?![-_. ]+[0-3][0-9])"#, true),
    (r#"^(?<title>.+?)[_. ](?<absoluteepisode>\d{1,4})(?:[_. ]+)(?:BLM|B[oö]l[uü]m)"#, true),
    (r#"^(?<title>.+?)?\W*(?<ambiguousairmonth>[0-1][0-9])[-_. ]+(?<ambiguousairday>[0-3][0-9])[-_. ]+(?<airyear>\d{4})(?!\d+)"#, true),
    (r#"^(?<title>.+?)?\W*(?<ambiguousairday>[0-3][0-9])[-_. ]+(?<ambiguousairmonth>[0-1][0-9])[-_. ]+(?<airyear>\d{4})(?!\d+)"#, true),
    (r#"^(?<title>.+?)?\W*(?<!\d+)(?<airyear>\d{4})(?<airmonth>[0-1][0-9])(?<airday>[0-3][0-9])(?!\d+)"#, true),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:(?:_|-|\s|\.)+(?:e|ep)(?<absoluteepisode>(\d{3}|\d{4})(\.\d{1,2})?))+[-_. ].*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?<title>.+?)?(?:(?:[-_. ](?<![()\[!]))*(?<!\d{1,2}-)(?<season>(?<!\d+|\(|\[|e|x)\d{2})(?<episode>(?<!e|x)(?:[1-9][0-9]|[0][1-9])(?!p|i|\d+|\)|\]|\W\d+|\W(?:e|ep|x)\d+)))+([-_. ]+|$)(?!\\)"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ](?:Se\.(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:[-_ ]?afl\.)(?<episode>\d{1,3}(?!\d+))(?:(?:[-]|[-_ ]en[-_ ])(?<episode>\d{1,3}(?!\d+)))*))"#,
        true,
    ),
    (
        r#"^(?<title>.*?)(?:(?:[-_\W](?<![()\[!]))+S?(?<season>(?<!\d+)\d{1,2}(?!\d+))(?:(?:\-|[ex]){1,2}(?<episode>\d{1}))+)+(\W+|_|$)(?!\\)"#,
        true,
    ),
    (r#"^(?:Season(?:_|-|\s|\.)(?<season>(?<!\d+)\d{1,2}(?!\d+)))(?:_|-|\s|\.)(?<episode>(?<!\d+)\d{1,2}(?!\d+))"#, true),
    (r#"^(?:(?<season>(?<!\d+)(?:\d{1,2})(?!\d+))(?:-(?<episode>\d{2,3}(?!\d+))))"#, true),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:_|\s|\.)+(?:e|ep)(?<absoluteepisode>\d{2,3}(\.\d{1,2})?)-(?<absoluteepisode>(?<!\d+)\d{1,2}(\.\d{1,2})?(?!\d+|-)).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:(?:_|-|\s|\.)+(?:e|ep)(?<absoluteepisode>\d{2,4}(\.\d{1,2})?))+[-_. ].*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?<title>.+?)[-_. ](?:Episode)(?:[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+)))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)[_. ]+(?<absoluteepisode>(?<!\d+)\d{1,2}(\.\d{1,2})?(?!\d+))-(?<absoluteepisode>(?<!\d+)\d{1,2}(\.\d{1,2})?(?!\d+|-)).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)[-_. ]+(?:Episode|Episodio)(?:[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,4}(\.\d{1,2})?(?!\d+|[ip])))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:[-_. ]\[)(?:(?:-?)(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+|[ip])))+(?:\][-_. ]).*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:[-_. ]+(?<absoluteepisode>(?<!\d+)\d{2,4}(\.\d{1,2})?(?!\d+|[ip])))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ]?)?(?<title>.+?)(?:(?:[-_\W](?<![()\[!]))+(?<absoluteepisode>(?<!\d+)\d{2,3}(\.\d{1,2})?(?!\d+|[ip])))+.*?(?<hash>[(\[]\w{8}[)\]])?$"#,
        true,
    ),
    (r#"^(?<title>.+?)[-_. ](?<season>[0]?\d?)(?:(?<episode>\d{2}){2}(?!\d+))[-_. ]"#, true),
    (
        r#"^(?:\[(?<subgroup>.+?)\][-_. ])?(?<title>.+?)[-_. ]+?[\[(](?:S|Season|Saison|Series|Stagione)[-_. ]?(?<season>\d{1,2}(?![-_. ]?\d+))(?:[-_. )\]]|$)+(?<extras>EXTRAS|SUBPACK)?(?!\\)"#,
        true,
    ),
    (r#"^(?:S?(?<season>(?<!\d+)(?:\d{1,2}|\d{4})(?!\d+))(?:(?:[-_ ]?[ex])(?<episode>\d{1}(?!\d+))))"#, true),
];

/// `SpecialEpisodeTitleRegex`
const SPECIAL_EPISODE_TITLE: &[(&str, bool)] = &[
    (r#"\.S\d+E00\.(?<episodetitle>.+?)(?:\.(?:720p|1080p|2160p|HDTV|WEB|WEBRip|WEB-DL)\.|$)"#, true),
    (r#"\.S\d+\.Special\.(?<episodetitle>.+?)(?:\.(?:720p|1080p|2160p|HDTV|WEB|WEBRip|WEB-DL)\.|$)"#, true),
];

/// `RejectHashedReleasesRegexes`: names that are hashes or other noise, not releases.
const REJECT_HASHED: &[(&str, bool)] = &[
    (r#"^[0-9a-zA-Z]{32}"#, false),
    (r#"^[a-z0-9]{24}$"#, false),
    (r#"^[A-Z]{11}\d{3}$"#, false),
    (r#"^[a-z]{12}\d{3}$"#, false),
    (r#"^Backup_\d{5,}S\d{2}-\d{2}$"#, false),
    (r#"^123$"#, false),
    (r#"^abc$"#, true),
    (r#"^abc[-_. ]xyz"#, true),
    (r#"^b00bs$"#, true),
    (r#"^\d{6}_\d{2}$"#, false),
    (r#"^[0-9a-zA-Z]{30}"#, false),
    (r#"^[0-9a-zA-Z]{26}"#, false),
    (r#"^[0-9a-zA-Z]{39}"#, false),
    (r#"^[0-9a-zA-Z]{24}"#, false),
];
