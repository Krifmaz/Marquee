//! The upstream language tests (`LanguageParserFixture`, `IsoLanguagesFixture`), run against the Rust ports. The
//! cases are Sonarr's and Radarr's own NUnit `[TestCase]`s, turned into JSON by `scripts/upstream_cases.py`; each
//! test method checks what the upstream one checks, and every method of the fixtures must be checked here.
//!
//! The methods that go through `ParseTitle` (Sonarr) or `ParseMovieTitle` (Radarr) go through their ports.

use serde_json::Value;
use server::parser::language::{self, tv, Catalog, Language, MOVIE, TV};
use server::parser::{episode, language_movie, language_tv, movie};
use std::collections::BTreeMap;

/// One upstream fixture: test method → its cases.
type Fixture = BTreeMap<String, Vec<Vec<Value>>>;

/// Reads `server/tests/upstream/<app>/<name>.json`.
fn fixture(app: &str, name: &str) -> Fixture {
    let path = format!("{}/tests/upstream/{app}/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("valid JSON")
}

/// A case's string argument `i`, or `None` for `null`.
fn opt(case: &[Value], i: usize) -> Option<&str> {
    case.get(i).and_then(Value::as_str)
}

/// A case's string argument `i`.
fn s(case: &[Value], i: usize) -> &str {
    opt(case, i).unwrap_or_else(|| panic!("argument {i} of {case:?} is not a string"))
}

/// What one upstream test method checks.
enum Check {
    /// `ParseLanguages` contains all of these
    Contains(Vec<Language>),
    /// `ParseLanguages` is exactly these, in any order
    Exactly(Vec<Language>),
    /// `ParseLanguages` starts with this one
    First(Language),
    /// `ParseSubtitleLanguage` is this one
    Subtitle(Language),
    /// `ParseSubtitleLanguageInformation` gives the tags, title and language named in the case
    TitleAndTags,
    /// `ParseSubtitleLanguageInformation` finds no title: Unknown, no tags, no raw title
    FalseTitle,
}

/// The language a method name ends in (`should_parse_language_<name>`), looked up by name.
fn by_method(cat: &Catalog, rest: &str) -> Language {
    let name = match rest {
        "portuguese_brazil" | "brazilian_portuguese" => "Portuguese (Brazil)",
        "spanish_latino" => "Spanish (Latino)",
        "ukrainian_multi" => "Ukrainian",
        other => other,
    };
    cat.by_name(name).unwrap_or_else(|| panic!("no language named {name}"))
}

/// Runs every case of a `LanguageParserFixture` and fails with all mismatches.
fn run(app: &str, cat: &Catalog, parse: &dyn Fn(&str, &str) -> Vec<Language>, special: &[(&str, Check)]) {
    let f = fixture(app, "LanguageParserFixture");
    let (mut fails, mut total) = (Vec::new(), 0);
    for (method, cases) in &f {
        let check = match special.iter().find(|(m, _)| m == method) {
            Some((_, c)) => c,
            None => {
                let rest = method.strip_prefix("should_parse_language_").unwrap_or_else(|| panic!("{method} is not checked here"));
                &Check::Contains(vec![by_method(cat, rest)])
            }
        };
        assert!(!cases.is_empty(), "{method} has no cases");
        for case in cases {
            total += 1;
            let input = s(case, 0);
            let ok = match check {
                Check::Contains(want) => {
                    let got = parse(method, input);
                    want.iter().all(|l| got.contains(l))
                }
                Check::Exactly(want) => {
                    let got = parse(method, input);
                    got.len() == want.len() && want.iter().all(|l| got.contains(l))
                }
                Check::First(want) => parse(method, input).first() == Some(want),
                Check::Subtitle(want) => language::parse_subtitle_language(cat, input) == *want,
                Check::TitleAndTags => {
                    let info = language::parse_subtitle_language_information(cat, input);
                    let mut tags: Vec<&str> = case[1].as_array().expect("tags").iter().filter_map(Value::as_str).collect();
                    let mut got: Vec<&str> = info.language_tags.iter().map(String::as_str).collect();
                    tags.sort_unstable();
                    got.sort_unstable();
                    got == tags && info.title.as_deref() == opt(case, 2) && Some(info.language) == cat.by_name(s(case, 3))
                }
                Check::FalseTitle => {
                    let info = language::parse_subtitle_language_information(cat, input);
                    info.language == cat.unknown && info.language_tags.is_empty() && info.raw_title.is_none()
                }
            };
            if !ok {
                let got = match check {
                    Check::Subtitle(_) => format!("{:?}", language::parse_subtitle_language(cat, input)),
                    Check::TitleAndTags | Check::FalseTitle => {
                        format!("{:?}", language::parse_subtitle_language_information(cat, input))
                    }
                    _ => format!("{:?}", parse(method, input).iter().map(|l| l.name).collect::<Vec<_>>()),
                };
                fails.push(format!("{method}({input:?}) → {got}"));
            }
        }
    }
    eprintln!("{total} upstream cases checked");
    assert!(fails.is_empty(), "{} of {total} upstream cases failed:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn sonarr_language_parser_fixture() {
    use tv::*;
    run(
        "sonarr",
        &TV,
        &|method, t| {
            // as each upstream method calls it: `ParseLanguages`, or `ParseTitle` for these
            const TITLE: &[&str] = &[
                "should_add_original_language_to_german_release_with_dl_tag",
                "should_not_add_original_language_to_german_release_when_title_contains_web_dl",
                "should_add_original_language_and_english_to_german_release_with_ml_tag",
                "should_parse_original_title_from_release_name",
                "should_parse_english_and_ukranian",
                "should_parse_turkish_and_ukranian",
            ];
            if TITLE.contains(&method) {
                episode::parse_title(t).map(|p| p.languages).unwrap_or_default()
            } else {
                language_tv::parse_languages(t)
            }
        },
        &[
            ("should_parse_subtitle_language_unknown", Check::Subtitle(UNKNOWN)),
            ("should_parse_subtitle_language_english", Check::Subtitle(ENGLISH)),
            ("should_parse_hindi_and_english", Check::Exactly(vec![HINDI, ENGLISH])),
            ("should_parse_spanish_and_catalan", Check::Exactly(vec![SPANISH, CATALAN])),
            ("should_parse_english_spanish_and_catalan", Check::Exactly(vec![ENGLISH, SPANISH, CATALAN])),
            ("should_add_original_language_to_german_release_with_dl_tag", Check::Exactly(vec![GERMAN, ORIGINAL])),
            ("should_not_add_original_language_to_german_release_when_title_contains_web_dl", Check::Exactly(vec![GERMAN])),
            ("should_add_original_language_and_english_to_german_release_with_ml_tag", Check::Exactly(vec![GERMAN, ORIGINAL, ENGLISH])),
            ("should_parse_original_title_from_release_name", Check::Exactly(vec![ORIGINAL])),
            ("should_parse_english_and_ukranian", Check::Exactly(vec![UKRAINIAN, ENGLISH])),
            ("should_parse_turkish_and_ukranian", Check::Exactly(vec![UKRAINIAN, TURKISH])),
            ("should_parse_title_and_tags", Check::TitleAndTags),
            ("should_not_parse_false_title", Check::FalseTitle),
        ],
    );
}

#[test]
fn radarr_language_parser_fixture() {
    use server::parser::language::movie::*;
    run(
        "radarr",
        &MOVIE,
        &|method, t| {
            // as each upstream method calls it: `ParseLanguages`, `ParseMovieTitle(t)` or `ParseMovieTitle(t, true)`
            const DIRECT: &[&str] = &[
                "afrikaans",
                "albanian",
                "georgian",
                "kannada",
                "malayalam",
                "marathi",
                "mongolian",
                "romansh",
                "spanish_latino",
                "tagalog",
                "urdu",
            ];
            const NOT_DIR: &[&str] = &["arabic", "bengali", "latvian", "persian", "slovak", "telugu"];
            let lang = method.strip_prefix("should_parse_language_").unwrap_or("");
            if DIRECT.contains(&lang) {
                return language_movie::parse_languages(t);
            }
            let is_dir = method.starts_with("should_parse_language_") && !NOT_DIR.contains(&lang);
            movie::parse_movie_title(t, is_dir).map(|p| p.languages).unwrap_or_default()
        },
        &[
            ("should_parse_subtitle_language", Check::Subtitle(ENGLISH)),
            ("should_parse_subtitle_language_english", Check::Subtitle(ENGLISH)),
            ("should_parse_subtitle_language_unknown", Check::Subtitle(UNKNOWN)),
            ("should_parse_language_french_english", Check::Contains(vec![FRENCH, ENGLISH])),
            ("should_parse_language_polish_subbed", Check::Contains(vec![UNKNOWN])),
            ("should_parse_language_spanish_latino", Check::First(SPANISH_LATINO)),
            ("should_add_original_language_to_german_release_with_dl_tag", Check::Exactly(vec![GERMAN, ORIGINAL])),
            ("should_not_add_original_language_to_german_release_when_title_contains_web_dl", Check::Exactly(vec![GERMAN])),
            ("should_add_original_language_and_english_to_german_release_with_ml_tag", Check::Exactly(vec![GERMAN, ORIGINAL, ENGLISH])),
            ("should_parse_original_title_from_release_name", Check::Exactly(vec![ORIGINAL])),
            ("should_parse_title_and_tags", Check::TitleAndTags),
            ("should_not_parse_false_title", Check::FalseTitle),
        ],
    );
}

/// Runs an `IsoLanguagesFixture`: each method names the language its codes must find, or none.
fn run_iso(app: &str, cat: &Catalog, special: &[(&str, Option<Language>)]) {
    let f = fixture(app, "IsoLanguagesFixture");
    let (mut fails, mut total) = (Vec::new(), 0);
    for (method, cases) in &f {
        let want = match special.iter().find(|(m, _)| m == method) {
            Some((_, w)) => *w,
            None => {
                let rest = method.strip_prefix("should_return_").unwrap_or_else(|| panic!("{method} is not checked here"));
                Some(by_method(cat, rest))
            }
        };
        for case in cases {
            total += 1;
            let got = cat.find_iso(s(case, 0)).map(|i| i.language);
            if got != want {
                fails.push(format!("{method}({:?}) → {got:?}", s(case, 0)));
            }
        }
    }
    eprintln!("{total} upstream cases checked");
    assert!(fails.is_empty(), "{} of {total} upstream cases failed:\n{}", fails.len(), fails.join("\n"));
}

#[test]
fn sonarr_iso_languages_fixture() {
    run_iso(
        "sonarr",
        &TV,
        &[
            ("should_return_iso_language_for_English", Some(tv::ENGLISH)),
            ("unknown_or_invalid_code_should_return_null", None),
            ("should_return_portuguese", Some(tv::PORTUGUESE)),
            ("should_not_return_portuguese", None),
            ("should_lookup_cze_via_alternate_iso_code_mapping", Some(tv::CZECH)),
        ],
    );
}

#[test]
fn radarr_iso_languages_fixture() {
    run_iso(
        "radarr",
        &MOVIE,
        &[
            ("should_return_iso_language_for_English", Some(server::parser::language::movie::ENGLISH)),
            ("unknown_or_invalid_code_should_return_null", None),
            ("should_return_portuguese", Some(server::parser::language::movie::PORTUGUESE)),
            ("should_not_return_portuguese", None),
        ],
    );
}
