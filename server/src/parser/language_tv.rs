//! Sonarr's `LanguageParser.ParseLanguages`: the languages a TV release name says. Port of
//! `src/NzbDrone.Core/Parser/LanguageParser.cs` at the Sonarr commit in `UPSTREAM.lock`; the expressions are the
//! upstream ones, unchanged. The subtitle half is shared with Radarr in [`super::language`].

use super::language::{finish, named, tv as l, Language};
use super::net_regex::{NetRegex, I, NONE};
use std::sync::LazyLock;

/// `CleanSeriesTitleRegex`: drops the series title, so a title such as "The Danish Girl" doesn't count.
static CLEAN_SERIES_TITLE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r".*?[_. ](S\d{2}(?:E\d{2,4})*[_. ].*)", I));

/// `LanguageRegex`
static LANGUAGE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?:\W|_)(?<english>\b(?:ing|eng)\b)|(?<italian>\b(?:ita|italian)\b)|(?<german>german\b|videomann|ger[. ]dub)|(?<flemish>flemish)|(?<greek>greek)|(?<french>(?:\W|_)(?:FR|VF|VF2|VFF|VFI|VFQ|TRUEFRENCH|FRENCH)(?:\W|_))|(?<russian>\b(?:rus|ru)\b)|(?<hungarian>\b(?:HUNDUB|HUN)\b)|(?<hebrew>\bHebDub\b)|(?<polish>\b(?:PL\W?DUB|DUB\W?PL|LEK\W?PL|PL\W?LEK)\b)|(?<chinese>\[(?:CH[ST]|BIG5|GB)\]|简|繁|字幕)|(?<bulgarian>\bbgaudio\b)|(?<spanish>\b(?:español|castellano|esp|spa(?!\(Latino\)))\b)|(?<ukrainian>\b(?:\dx?)?(?:ukr))|(?<thai>\b(?:THAI)\b)|(?<romanian>\b(?:RoDubbed|ROMANIAN)\b)|(?<catalan>[-,. ]cat[. ](?:DD|subs)|\b(?:catalan|catalán)\b)|(?<latvian>\b(?:lat|lav|lv)\b)|(?<turkish>\b(?:tur)\b)|(?<original>\b(?:orig|original)\b)",
        I,
    )
});

/// `CaseSensitiveLanguageRegex`
static CASE_SENSITIVE_LANGUAGE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?:(?i)(?<!SUB[\W|_|^]))(?:(?<lithuanian>\bLT\b)|(?<czech>\bCZ\b)|(?<polish>\bPL\b)|(?<bulgarian>\bBG\b)|(?<slovak>\bSK\b))(?:(?i)(?![\W|_|^]SUB))",
        NONE,
    )
});

/// The `lowerTitle.Contains(…)` checks before the expressions, in upstream order.
const NAMED: &[(&[&str], Language)] = &[
    (&["spanish"], l::SPANISH),
    (&["danish"], l::DANISH),
    (&["dutch"], l::DUTCH),
    (&["japanese"], l::JAPANESE),
    (&["icelandic"], l::ICELANDIC),
    (&["mandarin", "cantonese", "chinese"], l::CHINESE),
    (&["korean"], l::KOREAN),
    (&["russian"], l::RUSSIAN),
    (&["polish"], l::POLISH),
    (&["vietnamese"], l::VIETNAMESE),
    (&["swedish"], l::SWEDISH),
    (&["norwegian"], l::NORWEGIAN),
    (&["finnish"], l::FINNISH),
    (&["turkish"], l::TURKISH),
    (&["portuguese"], l::PORTUGUESE),
    (&["hungarian"], l::HUNGARIAN),
    (&["hebrew"], l::HEBREW),
    (&["arabic"], l::ARABIC),
    (&["hindi"], l::HINDI),
    (&["malayalam"], l::MALAYALAM),
    (&["ukrainian"], l::UKRAINIAN),
    (&["bulgarian"], l::BULGARIAN),
    (&["slovak"], l::SLOVAK),
    (&["brazilian", "dublado"], l::PORTUGUESE_BRAZIL),
    (&["latino"], l::SPANISH_LATINO),
    (&["latvian"], l::LATVIAN),
];

/// The case-sensitive groups and their languages (`RegexLanguage`, first match only).
const CASE_SENSITIVE: &[(&str, Language)] =
    &[("lithuanian", l::LITHUANIAN), ("czech", l::CZECH), ("polish", l::POLISH), ("bulgarian", l::BULGARIAN), ("slovak", l::SLOVAK)];

/// The case-insensitive groups and their languages, checked on every match (`RegexLanguage`). Upstream also asks
/// for a `dutch` group the expression doesn't have; it never matches, so it is left out.
const GROUPS: &[(&str, Language)] = &[
    ("english", l::ENGLISH),
    ("italian", l::ITALIAN),
    ("german", l::GERMAN),
    ("flemish", l::FLEMISH),
    ("greek", l::GREEK),
    ("french", l::FRENCH),
    ("russian", l::RUSSIAN),
    ("hungarian", l::HUNGARIAN),
    ("hebrew", l::HEBREW),
    ("polish", l::POLISH),
    ("chinese", l::CHINESE),
    ("bulgarian", l::BULGARIAN),
    ("ukrainian", l::UKRAINIAN),
    ("spanish", l::SPANISH),
    ("thai", l::THAI),
    ("romanian", l::ROMANIAN),
    ("catalan", l::CATALAN),
    ("latvian", l::LATVIAN),
    ("turkish", l::TURKISH),
    ("original", l::ORIGINAL),
];

/// The languages a TV release name says (`ParseLanguages`); `[Unknown]` when it says none.
pub fn parse_languages(title: &str) -> Vec<Language> {
    let title = if CLEAN_SERIES_TITLE.is_match(title) { CLEAN_SERIES_TITLE.replace_all(title, "$1") } else { title.to_string() };
    let lower = title.to_lowercase();
    let mut languages = named(&lower, NAMED);
    if let Some(m) = CASE_SENSITIVE_LANGUAGE.captures(&title) {
        languages.extend(CASE_SENSITIVE.iter().filter(|(g, _)| m.ok(g)).map(|(_, lang)| *lang));
    }
    for m in LANGUAGE.captures_iter(&title) {
        languages.extend(GROUPS.iter().filter(|(g, _)| m.ok(g)).map(|(_, lang)| *lang));
    }
    if lower.contains("english") {
        languages.push(l::ENGLISH);
    }
    finish(languages, &title, l::UNKNOWN, l::GERMAN, l::ORIGINAL, l::ENGLISH)
}
