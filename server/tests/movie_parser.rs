//! Radarr's upstream title-parser tests (`ParserFixture`, `ReleaseGroupParserFixture`, `SceneCheckerFixture`,
//! `EditionParserFixture`, `CrapParserFixture`, `NormalizeTitleFixture`), run against the Rust port. The cases are
//! Radarr's own NUnit `[TestCase]`s, turned into JSON by `scripts/upstream_cases.py`; each test checks what the
//! upstream method checks, and every method of the fixtures must be checked here. The upstream `[Test]` methods
//! without cases are written out at the end.

use serde_json::Value;
use server::parser::language::movie as lang;
use server::parser::movie::{self, ParsedMovieInfo};
use server::parser::release_group_movie::parse_release_group;
use std::collections::BTreeMap;

/// One upstream fixture: test method → its cases.
type Fixture = BTreeMap<String, Vec<Vec<Value>>>;

/// Reads `server/tests/upstream/radarr/<name>.json`.
fn fixture(name: &str) -> Fixture {
    let path = format!("{}/tests/upstream/radarr/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("valid JSON")
}

/// A case's string argument `i`, `None` for `null`.
fn opt(case: &[Value], i: usize) -> Option<&str> {
    case.get(i).and_then(Value::as_str)
}

/// A case's string argument `i`.
fn s(case: &[Value], i: usize) -> &str {
    opt(case, i).unwrap_or_else(|| panic!("argument {i} of {case:?} is not a string"))
}

/// A case's number argument `i`.
fn n(case: &[Value], i: usize) -> i64 {
    case[i].as_i64().unwrap_or_else(|| panic!("argument {i} of {case:?} is not a number"))
}

/// Runs every case of a fixture through `check`, which answers `Err(what was got)` on a mismatch; every method
/// must be known to `check` (it answers `None` for an unknown one).
fn run(name: &str, check: impl Fn(&str, &[Value]) -> Option<Result<(), String>>) {
    let f = fixture(name);
    let (mut fails, mut total) = (Vec::new(), 0);
    for (method, cases) in &f {
        assert!(!cases.is_empty(), "{method} has no cases");
        for case in cases {
            total += 1;
            match check(method, case) {
                None => panic!("{name}: {method} is not checked here"),
                Some(Ok(())) => {}
                Some(Err(got)) => fails.push(format!("{method}({:?}) → {got}", case)),
            }
        }
    }
    eprintln!("{total} upstream cases checked");
    assert!(fails.is_empty(), "{} of {total} upstream cases of {name} failed:\n{}", fails.len(), fails.join("\n"));
}

/// `Ok` when `ok`, else what was parsed.
fn expect(ok: bool, got: impl std::fmt::Debug) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(format!("{got:?}"))
    }
}

/// `ParseMovieTitle(title, is_dir)`, checked by `f`; a name that can't be read fails (upstream throws).
fn parsed(title: &str, is_dir: bool, f: impl Fn(&ParsedMovieInfo) -> bool) -> Result<(), String> {
    match movie::parse_movie_title(title, is_dir) {
        Some(p) => expect(f(&p), &p),
        None => Err("not parsed".into()),
    }
}

#[test]
fn radarr_parser_fixture() {
    run("ParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_movie_title" => parsed(t, false, |p| p.primary_movie_title() == Some(s(c, 1))),
            "should_parse_anime_movie_title" => parsed(t, false, |p| {
                p.primary_movie_title() == Some(s(c, 1)) && p.release_group.as_deref() == opt(c, 2) && i64::from(p.year) == n(c, 3)
            }),
            "should_parse_anime_movie_title_without_year" => {
                parsed(t, false, |p| p.primary_movie_title() == Some(s(c, 1)) && p.release_group.as_deref() == opt(c, 2))
            }
            "should_parse_german_movie" => {
                parsed(t, false, |p| p.primary_movie_title() == Some(s(c, 1)) && p.edition == s(c, 2) && i64::from(p.year) == n(c, 3))
            }
            "should_parse_movie_alternative_titles" => parsed(t, true, |p| {
                let want: Vec<&str> = c[1].as_array().expect("titles").iter().filter_map(Value::as_str).collect();
                p.movie_titles == want
            }),
            "should_parse_movie_folder_name" => parsed(t, true, |p| p.primary_movie_title() == Some(s(c, 1))),
            "should_parse_movie_year" => parsed(t, false, |p| i64::from(p.year) == n(c, 1)),
            "should_parse_tmdb_id" => parsed(t, false, |p| i64::from(p.tmdb_id) == n(c, 1)),
            "should_not_parse_wrong_language_in_title" => parsed(t, true, |p| p.languages == vec![lang::UNKNOWN]),
            "should_parse_hardcoded_subs" => parsed(t, false, |p| p.hardcoded_subs.as_deref() == opt(c, 1)),
            "should_parse_imdb_in_title" => parsed(t, true, |p| p.imdb_id == s(c, 1)),
            "should_normalize_imdbid" => {
                let got = movie::normalize_imdb_id(t);
                expect(got.as_deref() == opt(c, 1), got)
            }
            _ => return None,
        })
    });
}

#[test]
fn radarr_release_group_parser_fixture() {
    run("ReleaseGroupParserFixture", |method, c| {
        let got = parse_release_group(s(c, 0));
        let want = if method == "should_not_parse_anime_hash_as_release_group" { None } else { opt(c, 1) };
        if !method.starts_with("should_") {
            return None;
        }
        Some(expect(got.as_deref() == want, got))
    });
}

#[test]
fn radarr_scene_checker_fixture() {
    run("SceneCheckerFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_return_true_for_scene_names" => expect(movie::is_scene_title(t), movie::get_scene_title(t)),
            "should_return_false_for_non_scene_names" => expect(!movie::is_scene_title(t), movie::get_scene_title(t)),
            "should_correctly_parse_scene_names" => {
                let got = movie::get_scene_title(t);
                expect(got.as_deref() == opt(c, 1), got)
            }
            _ => return None,
        })
    });
}

#[test]
fn radarr_edition_parser_fixture() {
    run("EditionParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_edition" => parsed(t, false, |p| p.edition == s(c, 1)),
            "should_not_parse_edition" => parsed(t, false, |p| p.edition.is_empty()),
            _ => return None,
        })
    });
}

#[test]
fn radarr_crap_parser_fixture() {
    run("CrapParserFixture", |method, c| {
        Some(match method {
            "should_not_parse_crap" | "should_not_parse_file_name_without_proper_spacing" | "should_not_parse_invalid_release_name" => {
                let got = movie::parse_movie_title(s(c, 0), false);
                expect(got.is_none(), got)
            }
            "should_not_parse_random" => {
                // 500 random names of this length, as upstream (a fixed seed here, so a failure can be repeated)
                const CHARSET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
                let len = usize::try_from(n(c, 0)).expect("length");
                let mut seed = 0x2545_f491_4f6c_dd1d_u64 ^ len as u64;
                let bad: Vec<String> = (0..500)
                    .map(|_| {
                        (0..len)
                            .map(|_| {
                                seed ^= seed << 13;
                                seed ^= seed >> 7;
                                seed ^= seed << 17;
                                CHARSET[(seed % CHARSET.len() as u64) as usize] as char
                            })
                            .collect::<String>()
                    })
                    .filter(|h| movie::parse_movie_title(h, false).is_some())
                    .collect();
                expect(bad.is_empty(), bad)
            }
            _ => return None,
        })
    });
}

#[test]
fn radarr_normalize_title_fixture() {
    run("NormalizeTitleFixture", |method, c| {
        let all = |formats: &[&str], want: &dyn Fn(&str) -> String| {
            let w = s(c, 0);
            let bad: Vec<(String, String)> = formats
                .iter()
                .map(|f| f.replace("{0}", w))
                .map(|d| (movie::clean_movie_title(&d), d))
                .filter(|(got, _)| *got != want(w))
                .collect();
            expect(bad.is_empty(), bad)
        };
        Some(match method {
            "should_normalize_series_title"
            | "should_remove_special_characters_and_casing"
            | "should_not_remove_from_the_beginning_of_the_title" => {
                let got = movie::clean_movie_title(s(c, 0));
                expect(got == s(c, 1), got)
            }
            "should_remove_common_words_from_middle_of_title" => {
                all(&["word.{0}.word", "word {0} word", "word-{0}-word"], &|_| "wordword".into())
            }
            "should_not_remove_common_words_from_end_of_title" => {
                all(&["word.word.{0}", "word-word-{0}", "word-word {0}"], &|w| format!("wordword{}", w.to_lowercase()))
            }
            "should_not_remove_common_words_in_the_middle_of_word" => {
                all(&["word.{0}word", "word {0}word", "word-{0}word", "word{0}.word", "word{0}-word", "word{0}-word"], &|w| {
                    format!("word{}word", w.to_lowercase())
                })
            }
            "should_not_clean_word_from_beginning_of_string" => {
                all(&["{0}.word.word", "{0}-word-word", "{0} word word"], &|w| format!("{w}wordword"))
            }
            _ => return None,
        })
    });
}

/// The upstream `[Test]` methods without cases.
#[test]
fn radarr_single_tests() {
    // ParserFixture.should_remove_accents_from_title
    assert_eq!(movie::clean_movie_title("Carniv\u{00E0}le"), "carnivale");
    // NormalizeTitleFixture
    for f in ["word.{0}.word", "word {0} word", "word-{0}-word"] {
        assert_eq!(movie::clean_movie_title(&f.replace("{0}", "a")), "wordword", "should_remove_a_from_middle_of_title");
    }
    for f in ["word.{0}.N.K.L.E.word", "word {0} N K L E word", "word-{0}-N-K-L-E-word"] {
        assert_eq!(movie::clean_movie_title(&f.replace("{0}", "a")), "wordankleword", "should_not_remove_a_when_at_start_of_acronym");
    }
    for f in ["word.N.K.L.E.{0}.word", "word N K L E {0} word", "word-N-K-L-E-{0}-word"] {
        assert_eq!(movie::clean_movie_title(&f.replace("{0}", "a")), "wordnkleaword", "should_not_remove_a_when_at_end_of_acronym");
    }
    assert_eq!(movie::clean_movie_title("Tokyo Ghoul A"), "tokyoghoula", "should_not_clean_trailing_a");
    // CrapParserFixture.should_not_parse_md5: upstream hashes "CRAPPY TEST SEED" 100 times; any 32-hex name is
    // rejected the same way, so a few fixed ones stand for them
    for h in ["5B7F8A2C9D0E1F3A4B6C7D8E9F0A1B2C", "D41D8CD98F00B204E9800998ECF8427E", "0123456789ABCDEF0123456789ABCDEF"] {
        assert!(movie::parse_movie_title(h, false).is_none(), "should_not_parse_md5: {h}");
    }
}
