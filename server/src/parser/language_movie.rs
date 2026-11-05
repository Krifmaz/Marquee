//! Radarr's `LanguageParser.ParseLanguages`: the languages a movie release name says. Port of
//! `src/NzbDrone.Core/Parser/LanguageParser.cs` at the Radarr commit in `UPSTREAM.lock`; the expressions are the
//! upstream ones, unchanged. The subtitle half is shared with Sonarr in [`super::language`].

use super::language::{finish, movie as l, named, Language};
use super::net_regex::{NetRegex, Opts, IX};
use std::sync::LazyLock;

/// `LanguageRegex`
static LANGUAGE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?:\W|_|^)(?<english>\beng\b)|
                                                                            (?<italian>\b(?:ita|italian)\b)|
                                                                            (?<german>(?:swiss)?german\b|videomann|ger[. ]dub|\bger\b)|
                                                                            (?<flemish>flemish)|
                                                                            (?<bulgarian>bgaudio)|
                                                                            (?<romanian>rodubbed)|
                                                                            (?<brazilian>\b(dublado|pt-BR)\b)|
                                                                            (?<greek>greek)|
                                                                            (?<french>\b(?:FR|VO|VF|VFF|VFQ|VFI|VF2|TRUEFRENCH|FRENCH|FRE|FRA)\b)|
                                                                            (?<russian>\b(?:rus|ru)\b)|
                                                                            (?<hungarian>\b(?:HUNDUB|HUN)\b)|
                                                                            (?<hebrew>\b(?:HebDub|HebDubbed)\b)|
                                                                            (?<polish>\b(?:PL\W?DUB|DUB\W?PL|LEK\W?PL|PL\W?LEK)\b)|
                                                                            (?<chinese>\[(?:CH[ST]|BIG5|GB)\]|简|繁|字幕)|
                                                                            (?<ukrainian>(?:(?:\dx)?UKR))|
                                                                            (?<spanish>\b(?:español|castellano)\b)|
                                                                            (?<catalan>\b(?:catalan?|catalán|català)\b)|
                                                                            (?<latvian>\b(?:lat|lav|lv)\b)|
                                                                            (?<telugu>\btel\b)|
                                                                            (?<vietnamese>\bVIE\b)|
                                                                            (?<japanese>\bJAP\b)|
                                                                            (?<korean>\bKOR\b)|
                                                                            (?<urdu>\burdu\b)|
                                                                            (?<romansh>\b(?:romansh|rumantsch|romansch)\b)|
                                                                            (?<mongolian>\b(?:mongolian|khalkha)\b)|
                                                                            (?<georgian>\b(?:georgian|geo|ka|kat)\b)|
                                                                            (?<original>\b(?:orig|original)\b)",
        IX,
    )
});

/// `CaseSensitiveLanguageRegex`
static CASE_SENSITIVE_LANGUAGE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?:(?i)(?<!SUB[\W|_|^]))(?:(?<english>\bEN\b)|
                                                                                                          (?<lithuanian>\bLT\b)|
                                                                                                          (?<czech>\bCZ\b)|
                                                                                                          (?<polish>\bPL\b)|
                                                                                                          (?<bulgarian>\bBG\b)|
                                                                                                          (?<slovak>\bSK\b)|
                                                                                                          (?<german>\bDE\b)|
                                                                                                          (?<spanish>\b(?<!DTS[._ -])ES\b))(?:(?i)(?![\W|_|^]SUB))",
        Opts { ignore_case: false, ignore_whitespace: true },
    )
});

/// The `lowerTitle.Contains(…)` checks before the expressions, in upstream order.
const NAMED: &[(&[&str], Language)] = &[
    (&["english"], l::ENGLISH),
    (&["spanish"], l::SPANISH),
    (&["danish"], l::DANISH),
    (&["dutch"], l::DUTCH),
    (&["japanese"], l::JAPANESE),
    (&["icelandic"], l::ICELANDIC),
    (&["mandarin", "cantonese", "chinese"], l::CHINESE),
    (&["korean"], l::KOREAN),
    (&["russian"], l::RUSSIAN),
    (&["romanian"], l::ROMANIAN),
    (&["hindi"], l::HINDI),
    (&["arabic"], l::ARABIC),
    (&["thai"], l::THAI),
    (&["bulgarian"], l::BULGARIAN),
    (&["polish"], l::POLISH),
    (&["vietnamese"], l::VIETNAMESE),
    (&["swedish"], l::SWEDISH),
    (&["norwegian"], l::NORWEGIAN),
    (&["finnish"], l::FINNISH),
    (&["turkish"], l::TURKISH),
    (&["portuguese"], l::PORTUGUESE),
    (&["brazilian"], l::PORTUGUESE_BR),
    (&["hungarian"], l::HUNGARIAN),
    (&["hebrew"], l::HEBREW),
    (&["ukrainian"], l::UKRAINIAN),
    (&["persian"], l::PERSIAN),
    (&["bengali"], l::BENGALI),
    (&["slovak"], l::SLOVAK),
    (&["latvian"], l::LATVIAN),
    (&["latino"], l::SPANISH_LATINO),
    (&["tamil"], l::TAMIL),
    (&["telugu"], l::TELUGU),
    (&["malayalam"], l::MALAYALAM),
    (&["kannada"], l::KANNADA),
    (&["albanian"], l::ALBANIAN),
    (&["afrikaans"], l::AFRIKAANS),
    (&["marathi"], l::MARATHI),
    (&["tagalog"], l::TAGALOG),
];

/// The case-sensitive groups and their languages, checked on every match.
const CASE_SENSITIVE: &[(&str, Language)] = &[
    ("english", l::ENGLISH),
    ("lithuanian", l::LITHUANIAN),
    ("czech", l::CZECH),
    ("polish", l::POLISH),
    ("bulgarian", l::BULGARIAN),
    ("slovak", l::SLOVAK),
    ("spanish", l::SPANISH),
    ("german", l::GERMAN),
];

/// The case-insensitive groups and their languages, checked on every match. Upstream also asks for a `dutch`
/// group the expression doesn't have; it never matches, so it is left out.
const GROUPS: &[(&str, Language)] = &[
    ("english", l::ENGLISH),
    ("italian", l::ITALIAN),
    ("german", l::GERMAN),
    ("flemish", l::FLEMISH),
    ("greek", l::GREEK),
    ("french", l::FRENCH),
    ("russian", l::RUSSIAN),
    ("bulgarian", l::BULGARIAN),
    ("brazilian", l::PORTUGUESE_BR),
    ("hungarian", l::HUNGARIAN),
    ("hebrew", l::HEBREW),
    ("polish", l::POLISH),
    ("chinese", l::CHINESE),
    ("spanish", l::SPANISH),
    ("catalan", l::CATALAN),
    ("ukrainian", l::UKRAINIAN),
    ("latvian", l::LATVIAN),
    ("romanian", l::ROMANIAN),
    ("telugu", l::TELUGU),
    ("vietnamese", l::VIETNAMESE),
    ("japanese", l::JAPANESE),
    ("korean", l::KOREAN),
    ("urdu", l::URDU),
    ("romansh", l::ROMANSH),
    ("mongolian", l::MONGOLIAN),
    ("georgian", l::GEORGIAN),
    ("original", l::ORIGINAL),
];

/// The languages a movie release name says (`ParseLanguages`); `[Unknown]` when it says none.
pub fn parse_languages(title: &str) -> Vec<Language> {
    let lower = title.to_lowercase();
    let mut languages = named(&lower, NAMED);
    for m in CASE_SENSITIVE_LANGUAGE.captures_iter(title) {
        languages.extend(CASE_SENSITIVE.iter().filter(|(g, _)| m.ok(g)).map(|(_, lang)| *lang));
    }
    for m in LANGUAGE.captures_iter(title) {
        languages.extend(GROUPS.iter().filter(|(g, _)| m.ok(g)).map(|(_, lang)| *lang));
    }
    finish(languages, title, l::UNKNOWN, l::GERMAN, l::ORIGINAL, l::ENGLISH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_dual_language_adds_original() {
        assert_eq!(parse_languages("Movie.2020.GERMAN.DL.1080p"), vec![l::GERMAN, l::ORIGINAL]);
    }
}
