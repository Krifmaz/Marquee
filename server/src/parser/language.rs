//! The languages Sonarr and Radarr know and what both read from subtitle file names. Ports of
//! `NzbDrone.Core.Languages.Language`, `Parser/IsoLanguage(s)` and the subtitle half of `Parser/LanguageParser`
//! at the commits in `UPSTREAM.lock`; the release-name half is in [`super::language_tv`] and
//! [`super::language_movie`].
//!
//! Sonarr and Radarr number their languages differently (Radarr's Hindi is 26, Sonarr's 27), so each has its own
//! list ([`tv`], [`movie`]) and lookups take the [`Catalog`] of the app they are for.
//!
//! .NET keeps every repetition of a regex group (`Group.Captures`); the subtitle expressions repeat their tag and
//! code groups, so here the repeated part is captured whole and read again piece by piece (see
//! [`super::net_regex`]).

use super::net_regex::{NetRegex, I, NONE};
use serde::Serialize;
use std::sync::LazyLock;

/// A language (`Language`): upstream compares languages by id only.
#[derive(Clone, Copy, Debug, Eq, Serialize)]
pub struct Language {
    /// upstream's number for it (`-2` is Original, `-1` Radarr's Any, `0` Unknown)
    pub id: i32,
    /// its English name, as the API and the UI show it
    pub name: &'static str,
}

impl PartialEq for Language {
    fn eq(&self, other: &Language) -> bool {
        self.id == other.id
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

/// A language's ISO codes (`IsoLanguage`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IsoLanguage {
    /// ISO 639-1 (`TwoLetterCode`)
    pub two: &'static str,
    /// the country, for the languages that have one (`CountryCode`: `pt-br`, `es-mx`)
    pub country: &'static str,
    /// ISO 639-2/T (`ThreeLetterCode`); empty for Radarr's Brazilian Portuguese
    pub three: &'static str,
    /// Radarr's English name (`EnglishName`); empty for Sonarr, which has none
    pub english_name: &'static str,
    /// the language
    pub language: Language,
}

/// One app's languages and ISO tables.
#[derive(Debug)]
pub struct Catalog {
    /// `Language.All`
    pub all: &'static [Language],
    /// `IsoLanguages.All`
    pub iso: &'static [IsoLanguage],
    /// `IsoLanguages.AlternateIsoCodeMappings`
    pub alternates: &'static [(&'static str, Language)],
    /// whether the alternate codes are looked up before the two-letter ones (Radarr) or after (Sonarr)
    pub alternates_first: bool,
    /// `FileNameBuilder.Iso639BTMap`
    pub bt_map: &'static [(&'static str, &'static str)],
    /// whether `FindByName` reads the ISO English name (Radarr) or the language name (Sonarr)
    pub by_english_name: bool,
    /// `Language.Unknown`
    pub unknown: Language,
}

/// Sonarr's languages.
pub static TV: Catalog = Catalog {
    all: tv::ALL,
    iso: tv::ISO,
    alternates: tv::ALTERNATES,
    alternates_first: false,
    bt_map: tv::BT_MAP,
    by_english_name: false,
    unknown: tv::UNKNOWN,
};

/// Radarr's languages.
pub static MOVIE: Catalog = Catalog {
    all: movie::ALL,
    iso: movie::ISO,
    alternates: movie::ALTERNATES,
    alternates_first: true,
    bt_map: movie::BT_MAP,
    by_english_name: true,
    unknown: movie::UNKNOWN,
};

impl Catalog {
    /// The language with this id (`FindById`); `None` for an id upstream doesn't know (it throws there).
    pub fn find_by_id(&self, id: i32) -> Option<Language> {
        if id == 0 {
            return Some(self.unknown);
        }
        self.all.iter().copied().find(|l| l.id == id)
    }

    /// The language with this name in any case (the `(Language)"name"` cast); `None` when unknown.
    pub fn by_name(&self, name: &str) -> Option<Language> {
        self.all.iter().copied().find(|l| l.name.to_lowercase() == name.to_lowercase())
    }

    /// The ISO entry for a code such as `en`, `pt-BR`, `eng` or `ger` (`IsoLanguages.Find`).
    pub fn find_iso(&self, iso_code: &str) -> Option<IsoLanguage> {
        let parts: Vec<&str> = iso_code.split('-').collect();
        let lang = parts[0].to_lowercase();
        let alternate = || self.alternates.iter().find(|(k, _)| *k == iso_code).and_then(|(_, l)| self.get_iso(*l));
        if self.alternates_first {
            if let Some(iso) = alternate() {
                return Some(iso);
            }
        }
        match lang.chars().count() {
            2 => {
                let mut found: Vec<&IsoLanguage> = self.iso.iter().filter(|l| l.two == lang).collect();
                if let Some(country) = parts.get(1).map(|c| c.to_lowercase()) {
                    found = if found.iter().any(|l| l.country == country) {
                        found.into_iter().filter(|l| l.country == country).collect()
                    } else {
                        found.into_iter().filter(|l| l.country.is_empty()).collect()
                    };
                }
                found.first().map(|l| **l)
            }
            _ if !self.alternates_first && self.alternates.iter().any(|(k, _)| *k == iso_code) => alternate(),
            3 => {
                let code = self.bt_map.iter().find(|(b, _)| *b == lang).map_or(lang.as_str(), |(_, t)| t);
                self.iso.iter().find(|l| l.three == code).copied()
            }
            _ => None,
        }
    }

    /// The first ISO entry of a language (`IsoLanguages.Get`).
    pub fn get_iso(&self, language: Language) -> Option<IsoLanguage> {
        self.iso.iter().find(|l| l.language == language).copied()
    }

    /// The ISO entry named `name` in any case (`IsoLanguages.FindByName`).
    pub fn find_iso_by_name(&self, name: &str) -> Option<IsoLanguage> {
        let name = name.trim().to_lowercase();
        self.iso.iter().copied().find(|l| {
            let own = if self.by_english_name { l.english_name } else { l.language.name };
            own.to_lowercase() == name
        })
    }
}

/// What a subtitle file name says (`SubtitleTitleInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleTitleInfo {
    /// `forced`, `default`, `sdh`, … in lower case (`LanguageTags`)
    pub language_tags: Vec<String>,
    /// the subtitle's language (`Language`)
    pub language: Language,
    /// the title as written, with its copy number (`RawTitle`)
    pub raw_title: Option<String>,
    /// the title without its copy number (`Title`)
    pub title: Option<String>,
    /// the copy number, 0 when there is none (`Copy`)
    pub copy: i32,
    /// whether the title comes before the tags (`TitleFirst`)
    pub title_first: bool,
}

/// The tag words of a subtitle file name.
const TAGS: &str = "forced|foreign|default|cc|psdh|sdh";

/// `SubtitleLanguageRegex`: `.+?([-_. ](?<tags>…))*[-_. ](?<iso_code>[a-z]{2,3})([-_. ](?<tags>…))*$`, with each
/// repeated part captured whole (`pre`, `post`) to read its repetitions.
static SUBTITLE_LANGUAGE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(&format!(r".+?(?<pre>(?:[-_. ](?:{TAGS}))*)[-_. ](?<iso_code>[a-z]{{2,3}})(?<post>(?:[-_. ](?:{TAGS}))*)$"), I)
});

/// One repetition of a `tags` group of [`SUBTITLE_LANGUAGE`].
static SUBTITLE_TAG: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(&format!(r"^[-_. ](?<tags>{TAGS})"), I));

/// `SubtitleLanguageTitleRegex`: `.+?(\.((?<tags1>…)|(?<iso_code>…)))*[-_. ](?<title>[^.]*)(\.((?<tags2>…)|
/// (?<iso_code>…)))*$`, with each repeated part captured whole (`pre`, `post`).
static SUBTITLE_LANGUAGE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        &format!(r".+?(?<pre>(?:\.(?:(?:{TAGS})|(?:[a-z]{{2,3}})))*)[-_. ](?<title>[^.]*)(?<post>(?:\.(?:(?:{TAGS})|(?:[a-z]{{2,3}})))*)$"),
        I,
    )
});

/// One repetition of [`SUBTITLE_LANGUAGE_TITLE`]'s repeated parts: a tag or an ISO code, up to the next dot.
static SUBTITLE_TITLE_PART: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(&format!(r"^\.(?:(?<tag>{TAGS})|(?<iso_code>[a-z]{{2,3}}))(?=\.|$)"), I));

/// `SubtitleTitleRegex`: a title and its copy number (`Title - 2`).
static SUBTITLE_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"^((?<title>.+) - )?(?<copy>(?<!\d+)\d{1,3}(?!\d+))$", NONE));

/// The file name without folders or extension (`Path.GetFileNameWithoutExtension`).
pub fn file_stem(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    file.rfind('.').map_or(file, |i| &file[..i])
}

/// Every repetition of `part` in `span`, which the repeated group covered, one after the other.
fn repetitions<'t>(part: &NetRegex, span: &'t str) -> Vec<super::net_regex::Caps<'t>> {
    let mut out = Vec::new();
    let mut rest = span;
    while let Some(c) = part.captures(rest) {
        let end = c.whole().end;
        if end == 0 {
            break;
        }
        rest = &rest[end..];
        out.push(c);
    }
    out
}

/// The language of a subtitle file (`ParseSubtitleLanguage`).
pub fn parse_subtitle_language(cat: &Catalog, file_name: &str) -> Language {
    let simple = file_stem(file_name);
    if let Some(m) = SUBTITLE_LANGUAGE.captures(simple) {
        return cat.find_iso(&m.str("iso_code").to_lowercase()).map_or(cat.unknown, |l| l.language);
    }
    let lower = simple.to_lowercase();
    cat.all.iter().copied().find(|l| lower.ends_with(&l.name.to_lowercase())).unwrap_or(cat.unknown)
}

/// The tags of a subtitle file name, in lower case (`ParseLanguageTags`).
pub fn parse_language_tags(file_name: &str) -> Vec<String> {
    let Some(m) = SUBTITLE_LANGUAGE.captures(file_stem(file_name)) else { return vec![] };
    [m.str("pre"), m.str("post")]
        .iter()
        .flat_map(|span| repetitions(&SUBTITLE_TAG, span))
        .map(|c| c.str("tags").to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

/// A subtitle's language and tags without a title (`ParseBasicSubtitle`).
pub fn parse_basic_subtitle(cat: &Catalog, file_name: &str) -> SubtitleTitleInfo {
    SubtitleTitleInfo {
        language_tags: parse_language_tags(file_name),
        language: parse_subtitle_language(cat, file_name),
        raw_title: None,
        title: None,
        copy: 0,
        title_first: false,
    }
}

/// A subtitle's language, tags and title (`ParseSubtitleLanguageInformation`); falls back to
/// [`parse_basic_subtitle`] when the name has no title or not exactly one language code.
pub fn parse_subtitle_language_information(cat: &Catalog, file_name: &str) -> SubtitleTitleInfo {
    let simple = file_stem(file_name);
    let Some(m) = SUBTITLE_LANGUAGE_TITLE.captures(simple) else { return parse_basic_subtitle(cat, file_name) };
    let pre = repetitions(&SUBTITLE_TITLE_PART, m.str("pre"));
    let post = repetitions(&SUBTITLE_TITLE_PART, m.str("post"));
    let codes: Vec<&str> = pre.iter().chain(&post).filter_map(|c| c.name("iso_code")).map(|p| p.text).collect();
    if !m.ok("title") || codes.len() != 1 {
        return parse_basic_subtitle(cat, file_name);
    }
    let language = cat.find_iso(&codes[0].to_lowercase()).map_or(cat.unknown, |l| l.language);
    let tags1: Vec<&str> = pre.iter().filter_map(|c| c.name("tag")).map(|p| p.text).collect();
    let tags2 = post.iter().filter_map(|c| c.name("tag")).map(|p| p.text);
    let mut info = SubtitleTitleInfo {
        title_first: tags1.is_empty(),
        language_tags: tags1.iter().copied().chain(tags2).filter(|t| !t.is_empty()).map(str::to_lowercase).collect(),
        raw_title: Some(m.str("title").to_string()),
        title: None,
        copy: 0,
        language,
    };
    update_title_and_copy_from_title(&mut info);
    info
}

/// Splits the raw title into title and copy number (`UpdateTitleAndCopyFromTitle`).
pub fn update_title_and_copy_from_title(info: &mut SubtitleTitleInfo) {
    match info.raw_title.as_deref() {
        None => {
            info.title = None;
            info.copy = 0;
        }
        Some(raw) => match SUBTITLE_TITLE.captures(raw) {
            Some(m) => {
                info.title = m.name("title").map(|p| p.text.to_string());
                info.copy = m.str("copy").parse().unwrap_or(0);
            }
            None => {
                info.title = Some(raw.to_string());
                info.copy = 0;
            }
        },
    }
}

/// `GermanDualLanguageRegex`
static GERMAN_DUAL: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(?<!WEB[-_. ]?)\bDL\b", I));
/// `GermanMultiLanguageRegex`
static GERMAN_MULTI: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\bML\b", I));

/// The end of both apps' `ParseLanguages`: Unknown when nothing was found, Original (and English) for German
/// releases tagged DL (ML), then each language once.
pub(super) fn finish(
    mut languages: Vec<Language>,
    title: &str,
    unknown: Language,
    german: Language,
    original: Language,
    english: Language,
) -> Vec<Language> {
    if languages.is_empty() {
        languages.push(unknown);
    }
    if languages.len() == 1 && languages[0] == german {
        if GERMAN_DUAL.is_match(title) {
            languages.push(original);
        } else if GERMAN_MULTI.is_match(title) {
            languages.push(original);
            languages.push(english);
        }
    }
    let mut out: Vec<Language> = Vec::with_capacity(languages.len());
    for l in languages {
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// The languages whose names appear in `lower` (the `lowerTitle.Contains(…)` checks), in table order.
pub(super) fn named(lower: &str, table: &[(&[&str], Language)]) -> Vec<Language> {
    table.iter().filter(|(words, _)| words.iter().any(|w| lower.contains(w))).map(|(_, l)| *l).collect()
}

/// Sonarr's languages (`Language`) and ISO codes (`IsoLanguages`).
pub mod tv {
    use super::{IsoLanguage, Language};
    /// Unknown
    pub const UNKNOWN: Language = Language { id: 0, name: "Unknown" };
    /// English
    pub const ENGLISH: Language = Language { id: 1, name: "English" };
    /// French
    pub const FRENCH: Language = Language { id: 2, name: "French" };
    /// Spanish
    pub const SPANISH: Language = Language { id: 3, name: "Spanish" };
    /// German
    pub const GERMAN: Language = Language { id: 4, name: "German" };
    /// Italian
    pub const ITALIAN: Language = Language { id: 5, name: "Italian" };
    /// Danish
    pub const DANISH: Language = Language { id: 6, name: "Danish" };
    /// Dutch
    pub const DUTCH: Language = Language { id: 7, name: "Dutch" };
    /// Japanese
    pub const JAPANESE: Language = Language { id: 8, name: "Japanese" };
    /// Icelandic
    pub const ICELANDIC: Language = Language { id: 9, name: "Icelandic" };
    /// Chinese
    pub const CHINESE: Language = Language { id: 10, name: "Chinese" };
    /// Russian
    pub const RUSSIAN: Language = Language { id: 11, name: "Russian" };
    /// Polish
    pub const POLISH: Language = Language { id: 12, name: "Polish" };
    /// Vietnamese
    pub const VIETNAMESE: Language = Language { id: 13, name: "Vietnamese" };
    /// Swedish
    pub const SWEDISH: Language = Language { id: 14, name: "Swedish" };
    /// Norwegian
    pub const NORWEGIAN: Language = Language { id: 15, name: "Norwegian" };
    /// Finnish
    pub const FINNISH: Language = Language { id: 16, name: "Finnish" };
    /// Turkish
    pub const TURKISH: Language = Language { id: 17, name: "Turkish" };
    /// Portuguese
    pub const PORTUGUESE: Language = Language { id: 18, name: "Portuguese" };
    /// Flemish
    pub const FLEMISH: Language = Language { id: 19, name: "Flemish" };
    /// Greek
    pub const GREEK: Language = Language { id: 20, name: "Greek" };
    /// Korean
    pub const KOREAN: Language = Language { id: 21, name: "Korean" };
    /// Hungarian
    pub const HUNGARIAN: Language = Language { id: 22, name: "Hungarian" };
    /// Hebrew
    pub const HEBREW: Language = Language { id: 23, name: "Hebrew" };
    /// Lithuanian
    pub const LITHUANIAN: Language = Language { id: 24, name: "Lithuanian" };
    /// Czech
    pub const CZECH: Language = Language { id: 25, name: "Czech" };
    /// Arabic
    pub const ARABIC: Language = Language { id: 26, name: "Arabic" };
    /// Hindi
    pub const HINDI: Language = Language { id: 27, name: "Hindi" };
    /// Bulgarian
    pub const BULGARIAN: Language = Language { id: 28, name: "Bulgarian" };
    /// Malayalam
    pub const MALAYALAM: Language = Language { id: 29, name: "Malayalam" };
    /// Ukrainian
    pub const UKRAINIAN: Language = Language { id: 30, name: "Ukrainian" };
    /// Slovak
    pub const SLOVAK: Language = Language { id: 31, name: "Slovak" };
    /// Thai
    pub const THAI: Language = Language { id: 32, name: "Thai" };
    /// Portuguese (Brazil)
    pub const PORTUGUESE_BRAZIL: Language = Language { id: 33, name: "Portuguese (Brazil)" };
    /// Spanish (Latino)
    pub const SPANISH_LATINO: Language = Language { id: 34, name: "Spanish (Latino)" };
    /// Romanian
    pub const ROMANIAN: Language = Language { id: 35, name: "Romanian" };
    /// Latvian
    pub const LATVIAN: Language = Language { id: 36, name: "Latvian" };
    /// Persian
    pub const PERSIAN: Language = Language { id: 37, name: "Persian" };
    /// Catalan
    pub const CATALAN: Language = Language { id: 38, name: "Catalan" };
    /// Croatian
    pub const CROATIAN: Language = Language { id: 39, name: "Croatian" };
    /// Serbian
    pub const SERBIAN: Language = Language { id: 40, name: "Serbian" };
    /// Bosnian
    pub const BOSNIAN: Language = Language { id: 41, name: "Bosnian" };
    /// Estonian
    pub const ESTONIAN: Language = Language { id: 42, name: "Estonian" };
    /// Tamil
    pub const TAMIL: Language = Language { id: 43, name: "Tamil" };
    /// Indonesian
    pub const INDONESIAN: Language = Language { id: 44, name: "Indonesian" };
    /// Macedonian
    pub const MACEDONIAN: Language = Language { id: 45, name: "Macedonian" };
    /// Slovenian
    pub const SLOVENIAN: Language = Language { id: 46, name: "Slovenian" };
    /// Original
    pub const ORIGINAL: Language = Language { id: -2, name: "Original" };
    /// Every language, in upstream order (`Language.All`).
    pub const ALL: &[Language] = &[
        UNKNOWN,
        ENGLISH,
        FRENCH,
        SPANISH,
        GERMAN,
        ITALIAN,
        DANISH,
        DUTCH,
        JAPANESE,
        ICELANDIC,
        CHINESE,
        RUSSIAN,
        POLISH,
        VIETNAMESE,
        SWEDISH,
        NORWEGIAN,
        FINNISH,
        TURKISH,
        PORTUGUESE,
        FLEMISH,
        GREEK,
        KOREAN,
        HUNGARIAN,
        HEBREW,
        LITHUANIAN,
        CZECH,
        ARABIC,
        HINDI,
        BULGARIAN,
        MALAYALAM,
        UKRAINIAN,
        SLOVAK,
        THAI,
        PORTUGUESE_BRAZIL,
        SPANISH_LATINO,
        ROMANIAN,
        LATVIAN,
        PERSIAN,
        CATALAN,
        CROATIAN,
        SERBIAN,
        BOSNIAN,
        ESTONIAN,
        TAMIL,
        INDONESIAN,
        MACEDONIAN,
        SLOVENIAN,
        ORIGINAL,
    ];
    /// The ISO codes of each language (`IsoLanguages.All`), in upstream order.
    pub const ISO: &[IsoLanguage] = &[
        IsoLanguage { two: "en", country: "", three: "eng", english_name: "", language: ENGLISH },
        IsoLanguage { two: "fr", country: "fr", three: "fra", english_name: "", language: FRENCH },
        IsoLanguage { two: "es", country: "", three: "spa", english_name: "", language: SPANISH },
        IsoLanguage { two: "de", country: "de", three: "deu", english_name: "", language: GERMAN },
        IsoLanguage { two: "it", country: "", three: "ita", english_name: "", language: ITALIAN },
        IsoLanguage { two: "da", country: "", three: "dan", english_name: "", language: DANISH },
        IsoLanguage { two: "nl", country: "", three: "nld", english_name: "", language: DUTCH },
        IsoLanguage { two: "ja", country: "", three: "jpn", english_name: "", language: JAPANESE },
        IsoLanguage { two: "is", country: "", three: "isl", english_name: "", language: ICELANDIC },
        IsoLanguage { two: "zh", country: "cn", three: "zho", english_name: "", language: CHINESE },
        IsoLanguage { two: "ru", country: "", three: "rus", english_name: "", language: RUSSIAN },
        IsoLanguage { two: "pl", country: "", three: "pol", english_name: "", language: POLISH },
        IsoLanguage { two: "vi", country: "", three: "vie", english_name: "", language: VIETNAMESE },
        IsoLanguage { two: "sv", country: "", three: "swe", english_name: "", language: SWEDISH },
        IsoLanguage { two: "no", country: "", three: "nor", english_name: "", language: NORWEGIAN },
        IsoLanguage { two: "nb", country: "", three: "nob", english_name: "", language: NORWEGIAN },
        IsoLanguage { two: "fi", country: "", three: "fin", english_name: "", language: FINNISH },
        IsoLanguage { two: "tr", country: "", three: "tur", english_name: "", language: TURKISH },
        IsoLanguage { two: "pt", country: "pt", three: "por", english_name: "", language: PORTUGUESE },
        IsoLanguage { two: "nl", country: "", three: "nld", english_name: "", language: FLEMISH },
        IsoLanguage { two: "el", country: "", three: "ell", english_name: "", language: GREEK },
        IsoLanguage { two: "ko", country: "", three: "kor", english_name: "", language: KOREAN },
        IsoLanguage { two: "hu", country: "", three: "hun", english_name: "", language: HUNGARIAN },
        IsoLanguage { two: "he", country: "", three: "heb", english_name: "", language: HEBREW },
        IsoLanguage { two: "lt", country: "", three: "lit", english_name: "", language: LITHUANIAN },
        IsoLanguage { two: "cs", country: "", three: "ces", english_name: "", language: CZECH },
        IsoLanguage { two: "ar", country: "", three: "ara", english_name: "", language: ARABIC },
        IsoLanguage { two: "hi", country: "", three: "hin", english_name: "", language: HINDI },
        IsoLanguage { two: "bg", country: "", three: "bul", english_name: "", language: BULGARIAN },
        IsoLanguage { two: "ml", country: "", three: "mal", english_name: "", language: MALAYALAM },
        IsoLanguage { two: "uk", country: "", three: "ukr", english_name: "", language: UKRAINIAN },
        IsoLanguage { two: "sk", country: "", three: "slk", english_name: "", language: SLOVAK },
        IsoLanguage { two: "th", country: "th", three: "tha", english_name: "", language: THAI },
        IsoLanguage { two: "pt", country: "br", three: "por", english_name: "", language: PORTUGUESE_BRAZIL },
        IsoLanguage { two: "es", country: "mx", three: "spa", english_name: "", language: SPANISH_LATINO },
        IsoLanguage { two: "ro", country: "", three: "ron", english_name: "", language: ROMANIAN },
        IsoLanguage { two: "lv", country: "", three: "lav", english_name: "", language: LATVIAN },
        IsoLanguage { two: "fa", country: "", three: "fas", english_name: "", language: PERSIAN },
        IsoLanguage { two: "ca", country: "", three: "cat", english_name: "", language: CATALAN },
        IsoLanguage { two: "hr", country: "", three: "hrv", english_name: "", language: CROATIAN },
        IsoLanguage { two: "sr", country: "", three: "srp", english_name: "", language: SERBIAN },
        IsoLanguage { two: "bs", country: "", three: "bos", english_name: "", language: BOSNIAN },
        IsoLanguage { two: "et", country: "", three: "est", english_name: "", language: ESTONIAN },
        IsoLanguage { two: "ta", country: "", three: "tam", english_name: "", language: TAMIL },
        IsoLanguage { two: "id", country: "", three: "ind", english_name: "", language: INDONESIAN },
        IsoLanguage { two: "mk", country: "", three: "mkd", english_name: "", language: MACEDONIAN },
        IsoLanguage { two: "sl", country: "", three: "slv", english_name: "", language: SLOVENIAN },
    ];
    /// Codes upstream maps to a language outside the ISO lists (`AlternateIsoCodeMappings`).
    pub const ALTERNATES: &[(&str, Language)] =
        &[("cze", CZECH), ("dut", DUTCH), ("mac", MACEDONIAN), ("rum", ROMANIAN), ("yue", CHINESE), ("zhtw", CHINESE)];
    /// ISO 639-2/B codes and their 639-2/T twins (`FileNameBuilder.Iso639BTMap`).
    pub const BT_MAP: &[(&str, &str)] = &[
        ("alb", "sqi"),
        ("arm", "hye"),
        ("baq", "eus"),
        ("bur", "mya"),
        ("chi", "zho"),
        ("cze", "ces"),
        ("dut", "nld"),
        ("fre", "fra"),
        ("geo", "kat"),
        ("ger", "deu"),
        ("gre", "ell"),
        ("ice", "isl"),
        ("mac", "mkd"),
        ("mao", "mri"),
        ("may", "msa"),
        ("per", "fas"),
        ("rum", "ron"),
        ("slo", "slk"),
        ("tib", "bod"),
        ("wel", "cym"),
    ];
}

/// Radarr's languages (`Language`) and ISO codes (`IsoLanguages`).
pub mod movie {
    use super::{IsoLanguage, Language};
    /// Unknown
    pub const UNKNOWN: Language = Language { id: 0, name: "Unknown" };
    /// English
    pub const ENGLISH: Language = Language { id: 1, name: "English" };
    /// French
    pub const FRENCH: Language = Language { id: 2, name: "French" };
    /// Spanish
    pub const SPANISH: Language = Language { id: 3, name: "Spanish" };
    /// German
    pub const GERMAN: Language = Language { id: 4, name: "German" };
    /// Italian
    pub const ITALIAN: Language = Language { id: 5, name: "Italian" };
    /// Danish
    pub const DANISH: Language = Language { id: 6, name: "Danish" };
    /// Dutch
    pub const DUTCH: Language = Language { id: 7, name: "Dutch" };
    /// Japanese
    pub const JAPANESE: Language = Language { id: 8, name: "Japanese" };
    /// Icelandic
    pub const ICELANDIC: Language = Language { id: 9, name: "Icelandic" };
    /// Chinese
    pub const CHINESE: Language = Language { id: 10, name: "Chinese" };
    /// Russian
    pub const RUSSIAN: Language = Language { id: 11, name: "Russian" };
    /// Polish
    pub const POLISH: Language = Language { id: 12, name: "Polish" };
    /// Vietnamese
    pub const VIETNAMESE: Language = Language { id: 13, name: "Vietnamese" };
    /// Swedish
    pub const SWEDISH: Language = Language { id: 14, name: "Swedish" };
    /// Norwegian
    pub const NORWEGIAN: Language = Language { id: 15, name: "Norwegian" };
    /// Finnish
    pub const FINNISH: Language = Language { id: 16, name: "Finnish" };
    /// Turkish
    pub const TURKISH: Language = Language { id: 17, name: "Turkish" };
    /// Portuguese
    pub const PORTUGUESE: Language = Language { id: 18, name: "Portuguese" };
    /// Flemish
    pub const FLEMISH: Language = Language { id: 19, name: "Flemish" };
    /// Greek
    pub const GREEK: Language = Language { id: 20, name: "Greek" };
    /// Korean
    pub const KOREAN: Language = Language { id: 21, name: "Korean" };
    /// Hungarian
    pub const HUNGARIAN: Language = Language { id: 22, name: "Hungarian" };
    /// Hebrew
    pub const HEBREW: Language = Language { id: 23, name: "Hebrew" };
    /// Lithuanian
    pub const LITHUANIAN: Language = Language { id: 24, name: "Lithuanian" };
    /// Czech
    pub const CZECH: Language = Language { id: 25, name: "Czech" };
    /// Hindi
    pub const HINDI: Language = Language { id: 26, name: "Hindi" };
    /// Romanian
    pub const ROMANIAN: Language = Language { id: 27, name: "Romanian" };
    /// Thai
    pub const THAI: Language = Language { id: 28, name: "Thai" };
    /// Bulgarian
    pub const BULGARIAN: Language = Language { id: 29, name: "Bulgarian" };
    /// Portuguese (Brazil)
    pub const PORTUGUESE_BR: Language = Language { id: 30, name: "Portuguese (Brazil)" };
    /// Arabic
    pub const ARABIC: Language = Language { id: 31, name: "Arabic" };
    /// Ukrainian
    pub const UKRAINIAN: Language = Language { id: 32, name: "Ukrainian" };
    /// Persian
    pub const PERSIAN: Language = Language { id: 33, name: "Persian" };
    /// Bengali
    pub const BENGALI: Language = Language { id: 34, name: "Bengali" };
    /// Slovak
    pub const SLOVAK: Language = Language { id: 35, name: "Slovak" };
    /// Latvian
    pub const LATVIAN: Language = Language { id: 36, name: "Latvian" };
    /// Spanish (Latino)
    pub const SPANISH_LATINO: Language = Language { id: 37, name: "Spanish (Latino)" };
    /// Catalan
    pub const CATALAN: Language = Language { id: 38, name: "Catalan" };
    /// Croatian
    pub const CROATIAN: Language = Language { id: 39, name: "Croatian" };
    /// Serbian
    pub const SERBIAN: Language = Language { id: 40, name: "Serbian" };
    /// Bosnian
    pub const BOSNIAN: Language = Language { id: 41, name: "Bosnian" };
    /// Estonian
    pub const ESTONIAN: Language = Language { id: 42, name: "Estonian" };
    /// Tamil
    pub const TAMIL: Language = Language { id: 43, name: "Tamil" };
    /// Indonesian
    pub const INDONESIAN: Language = Language { id: 44, name: "Indonesian" };
    /// Telugu
    pub const TELUGU: Language = Language { id: 45, name: "Telugu" };
    /// Macedonian
    pub const MACEDONIAN: Language = Language { id: 46, name: "Macedonian" };
    /// Slovenian
    pub const SLOVENIAN: Language = Language { id: 47, name: "Slovenian" };
    /// Malayalam
    pub const MALAYALAM: Language = Language { id: 48, name: "Malayalam" };
    /// Kannada
    pub const KANNADA: Language = Language { id: 49, name: "Kannada" };
    /// Albanian
    pub const ALBANIAN: Language = Language { id: 50, name: "Albanian" };
    /// Afrikaans
    pub const AFRIKAANS: Language = Language { id: 51, name: "Afrikaans" };
    /// Marathi
    pub const MARATHI: Language = Language { id: 52, name: "Marathi" };
    /// Tagalog
    pub const TAGALOG: Language = Language { id: 53, name: "Tagalog" };
    /// Urdu
    pub const URDU: Language = Language { id: 54, name: "Urdu" };
    /// Romansh
    pub const ROMANSH: Language = Language { id: 55, name: "Romansh" };
    /// Mongolian
    pub const MONGOLIAN: Language = Language { id: 56, name: "Mongolian" };
    /// Georgian
    pub const GEORGIAN: Language = Language { id: 57, name: "Georgian" };
    /// Any
    pub const ANY: Language = Language { id: -1, name: "Any" };
    /// Original
    pub const ORIGINAL: Language = Language { id: -2, name: "Original" };
    /// Every language, in upstream order (`Language.All`).
    pub const ALL: &[Language] = &[
        UNKNOWN,
        ENGLISH,
        FRENCH,
        SPANISH,
        GERMAN,
        ITALIAN,
        DANISH,
        DUTCH,
        JAPANESE,
        ICELANDIC,
        CHINESE,
        RUSSIAN,
        POLISH,
        VIETNAMESE,
        SWEDISH,
        NORWEGIAN,
        FINNISH,
        TURKISH,
        PORTUGUESE,
        FLEMISH,
        GREEK,
        KOREAN,
        HUNGARIAN,
        HEBREW,
        LITHUANIAN,
        CZECH,
        ROMANIAN,
        HINDI,
        THAI,
        BULGARIAN,
        PORTUGUESE_BR,
        ARABIC,
        UKRAINIAN,
        PERSIAN,
        BENGALI,
        SLOVAK,
        LATVIAN,
        SPANISH_LATINO,
        CATALAN,
        CROATIAN,
        SERBIAN,
        BOSNIAN,
        ESTONIAN,
        TAMIL,
        INDONESIAN,
        TELUGU,
        MACEDONIAN,
        SLOVENIAN,
        MALAYALAM,
        KANNADA,
        ALBANIAN,
        AFRIKAANS,
        MARATHI,
        TAGALOG,
        URDU,
        ROMANSH,
        MONGOLIAN,
        GEORGIAN,
        ANY,
        ORIGINAL,
    ];
    /// The ISO codes of each language (`IsoLanguages.All`), in upstream order.
    pub const ISO: &[IsoLanguage] = &[
        IsoLanguage { two: "en", country: "", three: "eng", english_name: "English", language: ENGLISH },
        IsoLanguage { two: "fr", country: "fr", three: "fra", english_name: "French", language: FRENCH },
        IsoLanguage { two: "es", country: "", three: "spa", english_name: "Spanish", language: SPANISH },
        IsoLanguage { two: "de", country: "de", three: "deu", english_name: "German", language: GERMAN },
        IsoLanguage { two: "it", country: "", three: "ita", english_name: "Italian", language: ITALIAN },
        IsoLanguage { two: "da", country: "", three: "dan", english_name: "Danish", language: DANISH },
        IsoLanguage { two: "nl", country: "", three: "nld", english_name: "Dutch", language: DUTCH },
        IsoLanguage { two: "ja", country: "", three: "jpn", english_name: "Japanese", language: JAPANESE },
        IsoLanguage { two: "is", country: "", three: "isl", english_name: "Icelandic", language: ICELANDIC },
        IsoLanguage { two: "zh", country: "cn", three: "zho", english_name: "Chinese", language: CHINESE },
        IsoLanguage { two: "ru", country: "", three: "rus", english_name: "Russian", language: RUSSIAN },
        IsoLanguage { two: "pl", country: "", three: "pol", english_name: "Polish", language: POLISH },
        IsoLanguage { two: "vi", country: "", three: "vie", english_name: "Vietnamese", language: VIETNAMESE },
        IsoLanguage { two: "sv", country: "", three: "swe", english_name: "Swedish", language: SWEDISH },
        IsoLanguage { two: "no", country: "", three: "nor", english_name: "Norwegian", language: NORWEGIAN },
        IsoLanguage { two: "nb", country: "", three: "nob", english_name: "Norwegian Bokmal", language: NORWEGIAN },
        IsoLanguage { two: "fi", country: "", three: "fin", english_name: "Finnish", language: FINNISH },
        IsoLanguage { two: "tr", country: "", three: "tur", english_name: "Turkish", language: TURKISH },
        IsoLanguage { two: "pt", country: "pt", three: "por", english_name: "Portuguese", language: PORTUGUESE },
        IsoLanguage { two: "el", country: "", three: "ell", english_name: "Greek", language: GREEK },
        IsoLanguage { two: "ko", country: "", three: "kor", english_name: "Korean", language: KOREAN },
        IsoLanguage { two: "hu", country: "", three: "hun", english_name: "Hungarian", language: HUNGARIAN },
        IsoLanguage { two: "he", country: "", three: "heb", english_name: "Hebrew", language: HEBREW },
        IsoLanguage { two: "cs", country: "", three: "ces", english_name: "Czech", language: CZECH },
        IsoLanguage { two: "hi", country: "", three: "hin", english_name: "Hindi", language: HINDI },
        IsoLanguage { two: "th", country: "", three: "tha", english_name: "Thai", language: THAI },
        IsoLanguage { two: "bg", country: "", three: "bul", english_name: "Bulgarian", language: BULGARIAN },
        IsoLanguage { two: "ro", country: "", three: "ron", english_name: "Romanian", language: ROMANIAN },
        IsoLanguage { two: "pt", country: "br", three: "", english_name: "Portuguese (Brazil)", language: PORTUGUESE_BR },
        IsoLanguage { two: "ar", country: "", three: "ara", english_name: "Arabic", language: ARABIC },
        IsoLanguage { two: "uk", country: "", three: "ukr", english_name: "Ukrainian", language: UKRAINIAN },
        IsoLanguage { two: "fa", country: "", three: "fas", english_name: "Persian", language: PERSIAN },
        IsoLanguage { two: "bn", country: "", three: "ben", english_name: "Bengali", language: BENGALI },
        IsoLanguage { two: "lt", country: "", three: "lit", english_name: "Lithuanian", language: LITHUANIAN },
        IsoLanguage { two: "sk", country: "", three: "slk", english_name: "Slovak", language: SLOVAK },
        IsoLanguage { two: "lv", country: "", three: "lav", english_name: "Latvian", language: LATVIAN },
        IsoLanguage { two: "es", country: "mx", three: "spa", english_name: "Spanish (Latino)", language: SPANISH_LATINO },
        IsoLanguage { two: "ca", country: "", three: "cat", english_name: "Catalan", language: CATALAN },
        IsoLanguage { two: "hr", country: "", three: "hrv", english_name: "Croatian", language: CROATIAN },
        IsoLanguage { two: "sr", country: "", three: "srp", english_name: "Serbian", language: SERBIAN },
        IsoLanguage { two: "bs", country: "", three: "bos", english_name: "Bosnian", language: BOSNIAN },
        IsoLanguage { two: "et", country: "", three: "est", english_name: "Estonian", language: ESTONIAN },
        IsoLanguage { two: "ta", country: "", three: "tam", english_name: "Tamil", language: TAMIL },
        IsoLanguage { two: "id", country: "", three: "ind", english_name: "Indonesian", language: INDONESIAN },
        IsoLanguage { two: "te", country: "", three: "tel", english_name: "Telugu", language: TELUGU },
        IsoLanguage { two: "mk", country: "", three: "mkd", english_name: "Macedonian", language: MACEDONIAN },
        IsoLanguage { two: "sl", country: "", three: "slv", english_name: "Slovenian", language: SLOVENIAN },
        IsoLanguage { two: "ml", country: "", three: "mal", english_name: "Malayalam", language: MALAYALAM },
        IsoLanguage { two: "kn", country: "", three: "kan", english_name: "Kannada", language: KANNADA },
        IsoLanguage { two: "sq", country: "", three: "sqi", english_name: "Albanian", language: ALBANIAN },
        IsoLanguage { two: "af", country: "", three: "afr", english_name: "Afrikaans", language: AFRIKAANS },
        IsoLanguage { two: "mr", country: "", three: "mar", english_name: "Marathi", language: MARATHI },
        IsoLanguage { two: "tl", country: "", three: "tgl", english_name: "Tagalog", language: TAGALOG },
        IsoLanguage { two: "ur", country: "", three: "urd", english_name: "Urdu", language: URDU },
        IsoLanguage { two: "rm", country: "", three: "roh", english_name: "Romansh", language: ROMANSH },
        IsoLanguage { two: "mn", country: "", three: "mon", english_name: "Mongolian", language: MONGOLIAN },
        IsoLanguage { two: "ka", country: "", three: "kat", english_name: "Georgian", language: GEORGIAN },
    ];
    /// Codes upstream maps to a language outside the ISO lists (`AlternateIsoCodeMappings`).
    pub const ALTERNATES: &[(&str, Language)] = &[("cn", CHINESE)];
    /// ISO 639-2/B codes and their 639-2/T twins (`FileNameBuilder.Iso639BTMap`).
    pub const BT_MAP: &[(&str, &str)] = &[
        ("alb", "sqi"),
        ("arm", "hye"),
        ("baq", "eus"),
        ("bur", "mya"),
        ("chi", "zho"),
        ("cze", "ces"),
        ("dut", "nld"),
        ("fre", "fra"),
        ("geo", "kat"),
        ("ger", "deu"),
        ("gre", "ell"),
        ("gsw", "deu"),
        ("ice", "isl"),
        ("mac", "mkd"),
        ("mao", "mri"),
        ("may", "msa"),
        ("per", "fas"),
        ("rum", "ron"),
        ("slo", "slk"),
        ("tib", "bod"),
        ("wel", "cym"),
        ("khk", "mon"),
        ("mvf", "mon"),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_codes_are_found_like_upstream() {
        assert_eq!(TV.find_iso("pt-br").unwrap().language, tv::PORTUGUESE_BRAZIL);
        assert_eq!(TV.find_iso("pt-PT").unwrap().language, tv::PORTUGUESE);
        assert!(TV.find_iso("pt-xx").is_none());
        assert_eq!(TV.find_iso("ger").unwrap().language, tv::GERMAN);
        assert_eq!(TV.find_iso("cze").unwrap().language, tv::CZECH);
        assert_eq!(MOVIE.find_iso("cn").unwrap().language, movie::CHINESE);
        assert!(TV.find_iso("cn").is_none());
        assert_eq!(MOVIE.find_iso_by_name(" Norwegian Bokmal").unwrap().two, "nb");
        assert_eq!(TV.find_by_id(27), Some(tv::HINDI));
        assert_eq!(MOVIE.find_by_id(26), Some(movie::HINDI));
    }

    #[test]
    fn file_stems_drop_folders_and_extension() {
        assert_eq!(file_stem("/a/b/Show.S01E01.eng.forced.srt"), "Show.S01E01.eng.forced");
        assert_eq!(file_stem("noext"), "noext");
    }
}
