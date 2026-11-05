//! Radarr's `Parser` and `SceneChecker`: the movie, year, edition, release group, languages and quality a release
//! or file name says. Port of `src/NzbDrone.Core/Parser/Parser.cs` and `SceneChecker.cs` at the Radarr commit in
//! `UPSTREAM.lock`; the expressions are the upstream ones, unchanged.

use super::language::Language;
use super::language_movie;
use super::net_regex::{NetRegex, I, IX, NONE};
use super::quality::{movie, MovieQuality, QualityModel};
use super::quality_movie;
use super::release_group_movie::{parse_release_group, remove_file_extension, CLEAN_TORRENT_SUFFIX, WEBSITE_POSTFIX, WEBSITE_PREFIX};
use super::text::{byte_at_utf16, dotnet_int, dotnet_long, remove_accent, utf16_len};
use std::sync::LazyLock;

/// What a movie release name says (`ParsedMovieInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedMovieInfo {
    /// the title and its alternative titles (`AKA`), the main one first (`MovieTitles`)
    pub movie_titles: Vec<String>,
    /// the name as given (`OriginalTitle`)
    pub original_title: String,
    /// the name without extension and trailing dashes (`ReleaseTitle`)
    pub release_title: String,
    /// the release title without `<>?*|` and with the movie title replaced by "A Movie" (`SimpleReleaseTitle`)
    pub simple_release_title: String,
    /// the quality (`Quality`)
    pub quality: QualityModel<MovieQuality>,
    /// the languages (`Languages`)
    pub languages: Vec<Language>,
    /// the release group (`ReleaseGroup`)
    pub release_group: Option<String>,
    /// the anime release hash, "" when none (`ReleaseHash`)
    pub release_hash: String,
    /// the edition, "" when none (`Edition`)
    pub edition: String,
    /// the year, 0 when none (`Year`)
    pub year: i32,
    /// the IMDb id, "" when none (`ImdbId`)
    pub imdb_id: String,
    /// the TMDb id, 0 when none (`TmdbId`)
    pub tmdb_id: i32,
    /// the burned-in subtitles (`HardcodedSubs`)
    pub hardcoded_subs: Option<String>,
}

impl ParsedMovieInfo {
    /// The main title (`PrimaryMovieTitle`).
    pub fn primary_movie_title(&self) -> Option<&str> {
        self.movie_titles.first().map(String::as_str)
    }
}

/// `EditionRegex`, as text: other expressions are built from it.
const EDITION: &str = r"\(?\b(?<edition>(((Recut.|Extended.|Ultimate.)?(Director.?s|Collector.?s|Theatrical|Ultimate|Extended|Despecialized|(Special|Rouge|Final|Assembly|Imperial|Diamond|Signature|Hunter|Rekall)(?=(.(Cut|Edition|Version)))|\d{2,3}(th)?.Anniversary)(?:.(Cut|Edition|Version))?(.(Extended|Uncensored|Remastered|Unrated|Uncut|Open.?Matte|IMAX|Fan.?Edit))?|((Uncensored|Remastered|Unrated|Uncut|Open?.Matte|IMAX|Fan.?Edit|Restored|((2|3|4)in1))))))\b\)?";

/// `ReportEditionRegex`
static REPORT_EDITION: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(&format!(r"^.+?{EDITION}"), I));

/// `HardcodedSubsRegex`
static HARDCODED_SUBS: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"\b((?<hcsub>(\w+(?<!SOFT|MULTI|HORRIBLE)SUBS?))|(?<hc>(HC|SUBBED)))\b", IX));

/// `ReportMovieTitleRegex`, in upstream order.
static REPORT_MOVIE_TITLE: LazyLock<Vec<NetRegex>> = LazyLock::new(|| {
    [
        // Anime [Subgroup] with year explicitly in parentheses, e.g. [Sub] Title 1993 (1993) [VHS]
        r"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>(?![(\[]).+?)\s*\((?<year>(1(8|9)|20)\d{2})\).*?(?<hash>\[\w{8}\])?(?:$|\.)".to_string(),
        // Anime [Subgroup] and Year
        r"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>(?![(\[]).+?)?(?:(?:[-_\W](?<![)\[!]))*(?<year>(1(8|9)|20)\d{2}(?!p|i|x|\d+|\]|\W\d+)))+.*?(?<hash>\[\w{8}\])?(?:$|\.)".to_string(),
        // Anime [Subgroup] no year, versioned title, hash
        r"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>(?![(\[]).+?)((v)(?:\d{1,2})(?:([-_. ])))(\[.*)?(?:[\[(][^])])?.*?(?<hash>\[\w{8}\])(?:$|\.)".to_string(),
        // Anime [Subgroup] no year, info in double sets of brackets, hash
        r"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>(?![(\[]).+?)(\[.*).*?(?<hash>\[\w{8}\])(?:$|\.)".to_string(),
        // Anime [Subgroup] no year, info in parentheses or brackets, hash
        r"^(?:\[(?<subgroup>.+?)\][-_. ]?)(?<title>(?![(\[]).+)(?:[\[(][^])]).*?(?<hash>\[\w{8}\])(?:$|\.)".to_string(),
        // Some german or french tracker formats (missing year, ...)
        format!(
            r"^(?<title>(?![(\[]).+?)((\W|_))({EDITION}.{{1,3}})?(?:(?<!(19|20)\d{{2}}.*?)(?<!(?:Good|The)[_ .-])(German|TrueFrench))(.+?)(?=((19|20)\d{{2}}|$))(?<year>(19|20)\d{{2}}(?!p|i|\d+|\]|\W\d+))?(\W+|_|$)(?!\\)"
        ),
        // Special, Despecialized, etc. Edition Movies, e.g: Mission.Impossible.3.Special.Edition.2011
        format!(
            r"^(?<title>(?![(\[]).+?)?(?:(?:[-_\W](?<![)\[!]))*{EDITION}.{{1,3}}(?<year>(1(8|9)|20)\d{{2}}(?!p|i|\d+|\]|\W\d+)))+(\W+|_|$)(?!\\)"
        ),
        // Normal movie format, e.g: Mission.Impossible.3.2011
        r"^(?<title>(?![(\[]).+?)?(?:(?:[-_\W](?<![)\[!]))*(?<year>(1(8|9)|20)\d{2}(?!p|i|(1(8|9)|20)\d{2}|\]|\W(1(8|9)|20)\d{2})))+(\W+|_|$)(?!\\)".to_string(),
        // PassThePopcorn Torrent names: Star.Wars[PassThePopcorn]
        r"^(?<title>.+?)?(?:(?:[-_\W](?<![()\[!]))*(?<year>(\[\w *\])))+(\W+|_|$)(?!\\)".to_string(),
        // That did not work? Maybe some tool uses [] for years. Who would do that?
        r"^(?<title>(?![(\[]).+?)?(?:(?:[-_\W](?<![)!]))*(?<year>(1(8|9)|20)\d{2}(?!p|i|\d+|\W\d+)))+(\W+|_|$)(?!\\)".to_string(),
        // As a last resort for movies that have ( or [ in their title.
        r"^(?<title>.+?)?(?:(?:[-_\W](?<![)\[!]))*(?<year>(1(8|9)|20)\d{2}(?!p|i|\d+|\]|\W\d+)))+(\W+|_|$)(?!\\)".to_string(),
    ]
    .iter()
    .map(|p| NetRegex::new(p, I))
    .collect()
});

/// `ReportMovieTitleFolderRegex`: folders that start with the year.
static REPORT_MOVIE_TITLE_FOLDER: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"^(?:(?:[-_\W](?<![)!]))*(?<year>(19|20)\d{2}(?!p|i|\d+|\W\d+)))+(\W+|_|$)(?<title>.+?)?$", NONE));

/// `RejectHashedReleasesRegex`
static REJECT_HASHED_RELEASES: LazyLock<Vec<NetRegex>> = LazyLock::new(|| {
    vec![
        // Generic match for md5 and mixed-case hashes.
        NetRegex::new(r"^[0-9a-zA-Z]{32}", NONE),
        // Generic match for shorter lower-case hashes.
        NetRegex::new(r"^[a-z0-9]{24}$", NONE),
        // Format seen on some NZBGeek releases
        NetRegex::new(r"^[A-Z]{11}\d{3}$", NONE),
        NetRegex::new(r"^[a-z]{12}\d{3}$", NONE),
        // Backup filename (Unknown origins)
        NetRegex::new(r"^Backup_\d{5,}S\d{2}-\d{2}$", NONE),
        // 123 - Started appearing December 2014
        NetRegex::new(r"^123$", NONE),
        // abc - Started appearing January 2015
        NetRegex::new(r"^abc$", I),
        // abc - Started appearing 2020
        NetRegex::new(r"^abc[-_. ]xyz", I),
        // b00bs - Started appearing January 2015
        NetRegex::new(r"^b00bs$", I),
    ]
});

/// `ReversedTitleRegex`: names written backwards.
static REVERSED_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(?:^|[-._ ])(p027|p0801)[-._ ]", NONE));
/// `AlternativeTitleRegex`: `AKA` or `/` between titles.
static ALTERNATIVE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"[ ]+(?:AKA|\/)[ ]+", I));
/// `BracketedAlternativeTitleRegex`: `Title (AKA Other)`.
static BRACKETED_ALTERNATIVE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(.*) \([ ]*AKA[ ]+(.*)\)", I));
/// `NormalizeAlternativeTitleRegex`: `A.K.A.`
static NORMALIZE_ALTERNATIVE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"[ ]+(?:A\.K\.A\.)[ ]+", I));
/// `NormalizeRegex`: small words and punctuation dropped when comparing titles.
static NORMALIZE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"((?:\b|_)(?<!^|[^a-zA-Z0-9_']\w[^a-zA-Z0-9_'])([aà](?!$|[^a-zA-Z0-9_']\w[^a-zA-Z0-9_'])|an|the|and|or|of)(?!$)(?:\b|_))|\W|_",
        I,
    )
});
/// `ReportImdbId`
static REPORT_IMDB_ID: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(?<imdbid>tt\d{7,8})", I));
/// `ReportTmdbId`
static REPORT_TMDB_ID: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"tmdb(id)?-(?<tmdbid>\d+)", I));
/// `SimpleTitleRegex`: resolutions, codecs and bit depths dropped before the title is read.
static SIMPLE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?:(480|540|576|720|1080|2160)[ip]|[xh][\W_]?26[45]|DD\W?5\W1|[<>?*]|848x480|1280x720|1920x1080|3840x2160|4096x2160|(8|10)b(it)?|10-bit)\s*?(?![a-b0-9])",
        I,
    )
});
/// `SimpleReleaseTitleRegex`
static SIMPLE_RELEASE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\s*(?:[<>?*|])", I));
/// `CleanQualityBracketsRegex`
static CLEAN_QUALITY_BRACKETS: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\[[a-z0-9 ._-]+\]$", I));
/// `SpecialCharRegex`
static SPECIAL_CHAR: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(\&|\:|\\|\/)+", NONE));
/// `PunctuationRegex`
static PUNCTUATION: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"[^\w\s]", NONE));
/// `ArticleWordRegex`
static ARTICLE_WORD: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"^(a|an|the)\s", I));
/// `SpecialEpisodeWordRegex`
static SPECIAL_EPISODE_WORD: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(part|special|edition|christmas)\b\s?", I));
/// `DuplicateSpacesRegex`
static DUPLICATE_SPACES: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\s{2,}", NONE));
/// `RequestInfoRegex`: `[request info]` in front.
static REQUEST_INFO: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"^(?:\[.+?\])+", NONE));
/// `MultiRegex`
static MULTI: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"[_. ](?<multi>multi)[_. ]", I));
/// `char.IsLetterOrDigit`
static LETTER_OR_DIGIT: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"[\p{L}\p{Nd}]", NONE));

/// The movie a file path says (`ParseMoviePath`): the file name, then folder and file name, then the folder.
pub fn parse_movie_path(path: &str) -> Option<ParsedMovieInfo> {
    let (dir, name) = match path.rsplit_once('/') {
        Some((d, n)) => (d.rsplit('/').next().unwrap_or(d), n),
        None => ("", path),
    };
    let ext = super::quality::path_extension(name);
    parse_movie_title(name, true)
        .or_else(|| parse_movie_title(&format!("{dir} {name}"), false))
        .or_else(|| parse_movie_title(&format!("{dir}{ext}"), false))
}

/// What a movie release or folder name says (`ParseMovieTitle`); `None` when it can't be read.
pub fn parse_movie_title(title: &str, is_dir: bool) -> Option<ParsedMovieInfo> {
    let original_title = title;
    if !validate_before_parsing(title) {
        return None;
    }
    let mut title = title.to_string();
    if REVERSED_TITLE.is_match(&title) {
        // Upstream interpolates the reversed char array, which .NET writes as its type name; kept as upstream does.
        let without = remove_file_extension(&title);
        let rest = byte_at_utf16(&title, utf16_len(&without)).map_or("", |i| &title[i..]).to_string();
        title = format!("System.Char[]{rest}");
    }
    let release_title = remove_file_extension(&title);
    let release_title = release_title.trim_matches(['-', '_']).replace('【', "[").replace('】', "]");
    // `ParserCommon.PreSubstitutionRegex` is empty in Radarr
    let simple_title = SIMPLE_TITLE.replace_all(&release_title, "");
    let simple_title = WEBSITE_PREFIX.replace_all(&simple_title, "");
    let simple_title = WEBSITE_POSTFIX.replace_all(&simple_title, "");
    let simple_title = CLEAN_TORRENT_SUFFIX.replace_all(&simple_title, "");
    let simple_title = CLEAN_QUALITY_BRACKETS.replace_all_with(&simple_title, |c| {
        let text = c.whole().text;
        if quality_movie::parse_quality_name(text).quality != movie::Unknown {
            String::new()
        } else {
            text.to_string()
        }
    });
    let folder = is_dir.then(|| &*REPORT_MOVIE_TITLE_FOLDER);
    for regex in REPORT_MOVIE_TITLE.iter().chain(folder) {
        let Some(m) = regex.captures(&simple_title) else { continue };
        let Some(mut result) = parse_movie_match(&m) else { continue };
        let mut simple_release_title = SIMPLE_RELEASE_TITLE.replace_all(&release_title, "");
        let replace = match m.name("title") {
            Some(t) => t.text.to_string(),
            None => result.primary_movie_title().unwrap_or_default().to_string(),
        };
        if !replace.trim().is_empty() {
            let stand_in = if replace.contains('.') { "A.Movie" } else { "A Movie" };
            match m.name("title") {
                Some(t) => {
                    // upstream cuts at the title's place in the simple title, counted in UTF-16 units
                    let start = utf16_len(&simple_title[..t.start]);
                    let len = utf16_len(t.text);
                    let a = byte_at_utf16(&simple_release_title, start)?;
                    let b = byte_at_utf16(&simple_release_title, start + len)?;
                    simple_release_title = format!("{}{stand_in}{}", &simple_release_title[..a], &simple_release_title[b..]);
                }
                None => simple_release_title = simple_release_title.replace(&replace, stand_in),
            }
        }
        result.release_group = parse_release_group(&simple_release_title);
        if let Some(sub) = m.name("subgroup").filter(|s| !s.text.trim().is_empty()) {
            result.release_group = Some(sub.text.to_string());
        }
        result.hardcoded_subs = parse_hardcode_subs(&title);
        let for_languages = match result.release_group.as_deref().filter(|g| !g.trim().is_empty()) {
            Some(g) => simple_release_title.replace(g, "RlsGrp"),
            None => simple_release_title.clone(),
        };
        result.languages = language_movie::parse_languages(&for_languages);
        result.quality = quality_movie::parse_quality(&title);
        if result.edition.trim().is_empty() {
            result.edition = parse_edition(&simple_release_title);
        }
        result.release_hash =
            m.name("hash").map(|h| h.text.trim_matches(['[', ']']).to_string()).filter(|h| h != "1280x720").unwrap_or_default();
        result.original_title = original_title.to_string();
        result.release_title = release_title.clone();
        result.imdb_id = parse_imdb_id(&simple_release_title);
        result.tmdb_id = parse_tmdb_id(&simple_release_title);
        result.simple_release_title = simple_release_title;
        return Some(result);
    }
    None
}

/// The IMDb id in a name, "" when none (`ParseImdbId`).
pub fn parse_imdb_id(title: &str) -> String {
    REPORT_IMDB_ID.captures(title).map(|m| m.str("imdbid").to_string()).filter(|id| id.len() == 9 || id.len() == 10).unwrap_or_default()
}

/// The TMDb id in a name, 0 when none (`ParseTmdbId`).
pub fn parse_tmdb_id(title: &str) -> i32 {
    REPORT_TMDB_ID.captures(title).and_then(|m| dotnet_int(m.str("tmdbid"))).unwrap_or(0)
}

/// The edition a name says, "" when none (`ParseEdition`).
pub fn parse_edition(title: &str) -> String {
    REPORT_EDITION
        .captures(title)
        .map(|m| m.str("edition").to_string())
        .filter(|e| !e.trim().is_empty())
        .map(|e| e.replace('.', " "))
        .unwrap_or_default()
}

/// German umlauts and ß written out (`ReplaceGermanUmlauts`).
pub fn replace_german_umlauts(s: &str) -> String {
    s.replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('Ä', "Ae").replace('Ö', "Oe").replace('Ü', "Ue").replace('ß', "ss")
}

/// An IMDb id as `tt` and at least seven digits (`NormalizeImdbId`); `None` when it isn't one.
pub fn normalize_imdb_id(imdb_id: &str) -> Option<String> {
    static IMDB: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"^(\d{1,10}|(tt)\d{1,10})$", NONE));
    if !IMDB.is_match(imdb_id) || imdb_id.len() <= 2 {
        return None;
    }
    Some(format!("tt{:0>7}", imdb_id.replace("tt", "")))
}

/// A title reduced for comparing (`CleanMovieTitle`): small words, punctuation, case, umlauts and accents gone;
/// a title of only digits stays.
pub fn clean_movie_title(title: &str) -> String {
    if title.trim().is_empty() || dotnet_long(title).is_some() {
        return title.to_string();
    }
    remove_accent(&replace_german_umlauts(&NORMALIZE.replace_all(title, "").to_lowercase()))
}

/// A title as words for searching (`NormalizeMovieTitle`).
pub fn normalize_movie_title(title: &str) -> String {
    let t = SPECIAL_EPISODE_WORD.replace_all(title, "");
    let t = PUNCTUATION.replace_all(&t, " ");
    DUPLICATE_SPACES.replace_all(&t, " ").trim().to_lowercase()
}

/// A title without punctuation, leading article or special characters (`NormalizeTitle`).
pub fn normalize_title(title: &str) -> String {
    let t = PUNCTUATION.replace_all(title, "");
    let t = ARTICLE_WORD.replace_all(&t, "");
    let t = DUPLICATE_SPACES.replace_all(&t, " ");
    SPECIAL_CHAR.replace_all(&t, "").trim().to_lowercase()
}

/// The name without `<>?*|` (`SimplifyReleaseTitle`).
pub fn simplify_release_title(title: &str) -> String {
    SIMPLE_RELEASE_TITLE.replace_all(title, "")
}

/// The burned-in subtitles a name says (`ParseHardcodeSubs`): the tag, or "Generic Hardcoded Subs".
pub fn parse_hardcode_subs(title: &str) -> Option<String> {
    let m = HARDCODED_SUBS.captures_iter(title).pop()?;
    if let Some(s) = m.name("hcsub") {
        return Some(s.text.to_string());
    }
    m.ok("hc").then(|| "Generic Hardcoded Subs".to_string())
}

/// Whether a name says it has several languages (`HasMultipleLanguages`).
pub fn has_multiple_languages(title: &str) -> bool {
    MULTI.is_match(title)
}

/// The title, year and edition of the first match (`ParseMovieMatchCollection`).
fn parse_movie_match(m: &super::net_regex::Caps<'_>) -> Option<ParsedMovieInfo> {
    let title = m.name("title")?;
    if title.text == "(" {
        return None;
    }
    let name = title.text.replace('_', " ");
    let name = NORMALIZE_ALTERNATIVE_TITLE.replace_all(&name, " AKA ");
    let name = REQUEST_INFO.replace_all(&name, "");
    let name = name.trim_matches(' ');
    let parts: Vec<&str> = name.split('.').collect();
    let mut movie_name = String::new();
    let mut previous_acronym = false;
    for (n, part) in parts.iter().enumerate() {
        let next = parts.get(n + 1).copied().unwrap_or("");
        let lower = part.to_lowercase();
        // a single letter of an acronym (U.N.C.L.E.), "a" inside one, or "Dr": kept with its dot
        let single_letter = utf16_len(part) == 1
            && lower != "a"
            && dotnet_int(part).is_none()
            && (previous_acronym || n < parts.len() - 1)
            && (previous_acronym || utf16_len(next) != 1 || dotnet_int(next).is_none());
        let acronym_a = lower == "a" && (previous_acronym || utf16_len(next) == 1);
        if single_letter || acronym_a || lower == "dr" {
            movie_name.push_str(part);
            movie_name.push('.');
            previous_acronym = true;
        } else {
            if previous_acronym {
                movie_name.push(' ');
                previous_acronym = false;
            }
            movie_name.push_str(part);
            movie_name.push(' ');
        }
    }
    let movie_name = movie_name.trim_matches(' ').to_string();
    let year = dotnet_int(m.str("year")).unwrap_or(0);
    let edition = m.name("edition").map(|e| e.text.replace('.', " ")).unwrap_or_default();
    let unbracketed = BRACKETED_ALTERNATIVE_TITLE.replace_all(&movie_name, "$1 AKA $2");
    let mut titles = vec![movie_name.clone()];
    titles
        .extend(ALTERNATIVE_TITLE.split(&unbracketed).into_iter().filter(|t| !t.trim().is_empty() && *t != movie_name).map(str::to_string));
    Some(ParsedMovieInfo {
        movie_titles: titles,
        original_title: String::new(),
        release_title: String::new(),
        simple_release_title: String::new(),
        quality: QualityModel::new(movie::Unknown),
        languages: vec![],
        release_group: None,
        release_hash: String::new(),
        edition,
        year,
        imdb_id: String::new(),
        tmdb_id: 0,
        hardcoded_subs: None,
    })
}

/// Whether a name is worth reading (`ValidateBeforeParsing`): not a password/yEnc post, has a letter or digit,
/// and isn't a known hash.
fn validate_before_parsing(title: &str) -> bool {
    let lower = title.to_lowercase();
    if lower.contains("password") && lower.contains("yenc") {
        return false;
    }
    if !LETTER_OR_DIGIT.is_match(title) {
        return false;
    }
    let without = remove_file_extension(title);
    !REJECT_HASHED_RELEASES.iter().any(|r| r.is_match(&without))
}

/// The scene name a title is, if it looks like one (`SceneChecker.GetSceneTitle`): dots, no spaces, and a
/// title, release group and quality that can be read.
pub fn get_scene_title(title: &str) -> Option<String> {
    if !title.contains('.') || title.contains(' ') {
        return None;
    }
    let parsed = parse_movie_title(title, false)?;
    if parsed.release_group.is_none()
        || parsed.quality.quality == movie::Unknown
        || parsed.primary_movie_title().is_none_or(|t| t.trim().is_empty())
        || parsed.release_title.trim().is_empty()
    {
        return None;
    }
    Some(parsed.release_title)
}

/// Whether a title is a scene name (`SceneChecker.IsSceneTitle`).
pub fn is_scene_title(title: &str) -> bool {
    get_scene_title(title).is_some()
}
