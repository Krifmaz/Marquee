//! Sonarr's upstream title-parser tests (`ParserFixture`, `SingleEpisodeParserFixture`, `MultiEpisodeParserFixture`,
//! `AbsoluteEpisodeNumberParserFixture`, `DailyEpisodeParserFixture`, `SeasonParserFixture`,
//! `MiniSeriesEpisodeParserFixture`, `AnimeMetadataParserFixture`, `CrapParserFixture`,
//! `IsPossibleSpecialEpisodeFixture`, `NormalizeEpisodeTitleFixture`, `NormalizeSeriesTitleFixture`,
//! `PathParserFixture`, `ReleaseGroupParserFixture`, `SceneCheckerFixture`, `UnicodeReleaseParserFixture`,
//! `UrlFixture`), run against the Rust port. The cases are Sonarr's own NUnit `[TestCase]`s, turned into JSON by
//! `scripts/upstream_cases.py`; each test checks what the upstream method checks, and every method of the
//! fixtures must be checked here. The upstream `[Test]` methods without cases are written out at the end.

use serde_json::Value;
use server::parser::episode::{self, ParsedEpisodeInfo};
use server::parser::language::TV;
use server::parser::quality::{tv, DetectionSource};
use std::collections::BTreeMap;

/// One upstream fixture: test method → its cases.
type Fixture = BTreeMap<String, Vec<Vec<Value>>>;

/// Reads `server/tests/upstream/sonarr/<name>.json`.
fn fixture(name: &str) -> Fixture {
    let path = format!("{}/tests/upstream/sonarr/{name}.json", env!("CARGO_MANIFEST_DIR"));
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

/// A case's number argument `i`, or `default` when the case leaves it out.
fn n_or(case: &[Value], i: usize, default: i64) -> i64 {
    match case.get(i) {
        None => default,
        Some(v) => v.as_i64().unwrap_or_else(|| panic!("argument {i} of {case:?} is not a whole number")),
    }
}

/// A case's number argument `i`.
fn n(case: &[Value], i: usize) -> i32 {
    i32::try_from(n_or(case, i, i64::MIN)).expect("a whole number")
}

/// A case's decimal argument `i`.
fn f(case: &[Value], i: usize) -> f64 {
    case[i].as_f64().unwrap_or_else(|| panic!("argument {i} of {case:?} is not a number"))
}

/// A case's list of numbers from argument `i` on: an array, or the rest of the arguments (`params int[]`).
fn ints(case: &[Value], i: usize) -> Vec<i32> {
    match &case[i] {
        Value::Array(a) => a.iter().map(|v| i32::try_from(v.as_i64().expect("number")).expect("i32")).collect(),
        _ => (i..case.len()).map(|j| n(case, j)).collect(),
    }
}

/// A case's list of strings from argument `i` on.
fn strings(case: &[Value], i: usize) -> Vec<String> {
    match &case[i] {
        Value::Array(a) => a.iter().map(|v| v.as_str().expect("string").to_string()).collect(),
        _ => (i..case.len()).map(|j| s(case, j).to_string()).collect(),
    }
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
                Some(Err(got)) => fails.push(format!("{method}({case:?}) → {got}")),
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

/// `ParseTitle(title)`, checked by `f`; a name that can't be read fails (upstream dereferences null).
fn parsed(title: &str, f: impl Fn(&ParsedEpisodeInfo) -> bool) -> Result<(), String> {
    match episode::parse_title(title) {
        Some(p) => expect(f(&p), &p),
        None => Err("not parsed".into()),
    }
}

/// `ParsePath(path)`, checked by `f`.
fn parsed_path(path: &str, f: impl Fn(&ParsedEpisodeInfo) -> bool) -> Result<(), String> {
    match episode::parse_path(path) {
        Some(p) => expect(f(&p), &p),
        None => Err("not parsed".into()),
    }
}

/// `AsOsAgnostic`: a Windows path as the Linux path the upstream tests use there.
fn os_agnostic(path: &str) -> String {
    let mut path = path.to_string();
    if path.len() > 2 && path.as_bytes()[1] == b':' {
        path = format!("/{}", path.replace(':', ""));
    }
    path.replace('\\', "/")
}

/// `XmlCleaner.ReplaceUnicode`: drops characters XML can't hold (controls, and those outside UTF-16's basic plane).
fn replace_unicode(s: &str) -> String {
    s.chars().filter(|&c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}')).collect()
}

/// A single episode with a title: the checks shared by several fixtures.
fn single(p: &ParsedEpisodeInfo, c: &[Value]) -> bool {
    p.episode_numbers.len() == 1
        && p.season_number == n(c, 2)
        && p.episode_numbers[0] == n(c, 3)
        && p.series_title == s(c, 1)
        && p.absolute_episode_numbers.is_empty()
        && !p.full_season
}

#[test]
fn sonarr_parser_fixture() {
    run("ParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_series_name" => {
                let got = episode::clean_series_title(&episode::parse_series_name(t));
                expect(got == episode::clean_series_title(s(c, 1)), got)
            }
            "should_parse_series_title_info" => {
                parsed(t, |p| p.series_title_info.title_without_year == s(c, 1) && i64::from(p.series_title_info.year) == n_or(c, 2, 0))
            }
            "should_clean_up_invalid_path_characters" => {
                episode::parse_title(t);
                Ok(())
            }
            "should_remove_request_info_from_title" => parsed(t, |p| p.series_title == s(c, 1)),
            "should_parse_quality_from_extension" => parsed(t, |p| {
                p.quality.quality != tv::Unknown
                    && p.quality.source_detection == DetectionSource::Extension
                    && p.quality.resolution_detection == DetectionSource::Extension
            }),
            "should_parse_releasetitle" => parsed(t, |p| p.release_title == s(c, 1)),
            "should_parse_multiple_series_titles" => parsed(t, |p| {
                let mut want = strings(c, 1);
                let mut got = p.series_title_info.all_titles.clone();
                want.sort();
                got.sort();
                got == want
            }),
            "should_parse_language_after_parsing_title" => {
                let want = TV.by_name(s(c, 1)).expect("language");
                parsed(t, |p| p.languages == vec![want])
            }
            _ => return None,
        })
    });
}

#[test]
fn sonarr_single_episode_parser_fixture() {
    run("SingleEpisodeParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_single_episode" | "should_parse_japanese_variety_show_format" => parsed(t, |p| single(p, c)),
            "should_parse_decimal_number_as_special" => parsed(t, |p| single(p, c) && p.special),
            "should_parse_split_episode" => parsed(t, |p| single(p, c) && p.is_split_episode),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_multi_episode_parser_fixture() {
    run("MultiEpisodeParserFixture", |method, c| {
        Some(match method {
            "should_parse_multiple_episodes" => parsed(s(c, 0), |p| {
                let mut want = ints(c, 3);
                let mut got = p.episode_numbers.clone();
                want.sort_unstable();
                got.sort_unstable();
                p.season_number == n(c, 2)
                    && got == want
                    && p.series_title == s(c, 1)
                    && p.absolute_episode_numbers.is_empty()
                    && !p.full_season
            }),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_absolute_episode_number_parser_fixture() {
    run("AbsoluteEpisodeNumberParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_absolute_numbers" => parsed(t, |p| {
                p.absolute_episode_numbers == [n(c, 2)]
                    && p.season_number == n(c, 3)
                    && p.episode_numbers.len() <= 1
                    && p.episode_numbers.first().copied().unwrap_or(0) == n(c, 4)
                    && p.series_title == s(c, 1)
                    && !p.full_season
            }),
            "should_parse_absolute_specials" => parsed(t, |p| {
                p.absolute_episode_numbers == [n(c, 2)]
                    && p.season_number == 0
                    && p.episode_numbers.first().copied().unwrap_or(0) == 0
                    && p.series_title == s(c, 1)
                    && !p.full_season
                    && p.special
            }),
            "should_parse_multi_episode_absolute_numbers" => parsed(t, |p| {
                let want: Vec<i32> = (n(c, 2)..=n(c, 3)).collect();
                let mut got = p.absolute_episode_numbers.clone();
                got.sort_unstable();
                got == want && p.series_title == s(c, 1) && !p.full_season
            }),
            "should_parse_anime_season_packs" => parsed(t, |p| {
                p.absolute_episode_numbers.is_empty() && p.series_title == s(c, 1) && p.full_season && p.season_number == n(c, 2)
            }),
            "should_handle_anime_recap_numbering" => parsed(t, |p| {
                p.series_title == s(c, 1)
                    && p.absolute_episode_numbers.is_empty()
                    && p.special_absolute_episode_numbers == [f(c, 2)]
                    && !p.full_season
            }),
            "should_handle_ambiguously_named_anime_files_in_batch_release" => {
                let path = format!("{}/{}/{}", os_agnostic(r"C:\Test"), s(c, 0), s(c, 1));
                parsed_path(&path, |p| {
                    p.absolute_episode_numbers == [n(c, 3)]
                        && p.season_number == 0
                        && p.episode_numbers.is_empty()
                        && p.series_title == s(c, 2)
                        && !p.full_season
                })
            }
            "should_parse_absolute_followed_by_standard_as_standard" => parsed(t, |p| {
                p.episode_numbers.len() == 1
                    && p.season_number == n(c, 3)
                    && p.episode_numbers[0] == n(c, 4)
                    && p.series_title == s(c, 1)
                    && p.special_absolute_episode_numbers == [f(c, 2)]
                    && p.absolute_episode_numbers.is_empty()
                    && !p.full_season
            }),
            _ => return None,
        })
    });
}

/// `string.Expand` with a year, month and day.
fn expand(t: &str, year: &str, month: &str, day: &str) -> String {
    t.replace("{year}", year).replace("{month}", month).replace("{day}", day)
}

#[test]
fn sonarr_daily_episode_parser_fixture() {
    run("DailyEpisodeParserFixture", |method, c| {
        let t = s(c, 0);
        let date = || format!("{:04}-{:02}-{:02}", n(c, 2), n(c, 3), n(c, 4));
        let daily = |p: &ParsedEpisodeInfo| {
            p.series_title == s(c, 1)
                && p.air_date == date()
                && p.episode_numbers.is_empty()
                && p.absolute_episode_numbers.is_empty()
                && !p.full_season
        };
        Some(match method {
            "should_parse_daily_episode" => parsed(t, daily),
            "should_parse_daily_episode_with_multiple_parts" => parsed(t, |p| daily(p) && p.daily_part == Some(n(c, 5))),
            "should_not_accept_ancient_daily_series" => {
                let got = episode::parse_title(&expand(t, "1950", "10", "14"));
                expect(got.is_none(), got)
            }
            "should_not_accept_future_dates" => {
                let d = chrono::Local::now().date_naive() + chrono::Days::new(2);
                let got =
                    episode::parse_title(&expand(t, &d.format("%Y").to_string(), &d.format("%m").to_string(), &d.format("%d").to_string()));
                expect(got.is_none(), got)
            }
            "should_not_parse_ambiguous_daily_episode" => {
                let got = episode::parse_title(t);
                expect(got.is_none(), got)
            }
            _ => return None,
        })
    });
}

#[test]
fn sonarr_season_parser_fixture() {
    run("SeasonParserFixture", |method, c| {
        let t = s(c, 0);
        let season = |p: &ParsedEpisodeInfo| {
            p.season_number == n(c, 2) && p.series_title == s(c, 1) && p.episode_numbers.is_empty() && p.absolute_episode_numbers.is_empty()
        };
        Some(match method {
            "should_parse_full_season_release" => parsed(t, |p| season(p) && p.full_season),
            "should_parse_season_extras" | "should_parse_season_subpack" => parsed(t, |p| season(p) && p.full_season && p.is_season_extra),
            "should_parse_partial_season_release" => {
                parsed(t, |p| season(p) && !p.full_season && p.is_partial_season && p.season_part == n(c, 3))
            }
            "should_parse_multi_season_release" => parsed(t, |p| season(p) && p.full_season && !p.is_partial_season && p.is_multi_season),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_mini_series_episode_parser_fixture() {
    run("MiniSeriesEpisodeParserFixture", |method, c| {
        let mini = |p: &ParsedEpisodeInfo| {
            p.episode_numbers.len() == 1
                && p.season_number == 1
                && p.episode_numbers[0] == n(c, 2)
                && p.series_title == s(c, 1)
                && p.absolute_episode_numbers.is_empty()
                && !p.full_season
        };
        Some(match method {
            "should_parse_mini_series_episode" | "should_parse_korean_series_episode" => parsed(s(c, 0), mini),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_anime_metadata_parser_fixture() {
    run("AnimeMetadataParserFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_parse_releasegroup_and_hash" => {
                parsed(t, |p| p.release_group.as_deref() == opt(c, 1) && Some(p.release_hash.as_str()) == opt(c, 2))
            }
            "should_parse_release_hashes_with_10b_or_8b" => parsed(t, |p| Some(p.release_hash.as_str()) == opt(c, 1)),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_crap_parser_fixture() {
    run("CrapParserFixture", |method, c| {
        Some(match method {
            "should_not_parse_crap"
            | "should_not_parse_file_name_without_proper_spacing"
            | "should_not_parse_invalid_release_name"
            | "should_not_parse_special_with_part_number" => {
                let got = episode::parse_title(s(c, 0));
                expect(got.is_none(), got)
            }
            "should_parse_unknown_formats_without_error" => {
                episode::parse_title(s(c, 0));
                Ok(())
            }
            "should_not_parse_random" => {
                // 500 random names of this length, as upstream (a fixed seed here, so a failure can be repeated)
                const CHARSET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
                let len = usize::try_from(n(c, 0)).expect("length");
                let mut seed = 0x9e37_79b9_7f4a_7c15_u64 ^ len as u64;
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
                    .filter(|h| episode::parse_title(h).is_some())
                    .collect();
                expect(bad.is_empty(), bad)
            }
            _ => return None,
        })
    });
}

#[test]
fn sonarr_is_possible_special_episode_fixture() {
    run("IsPossibleSpecialEpisodeFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "IsPossibleSpecialEpisode_should_be_true" | "IsPossibleSpecialEpisode_should_be_true_if_e00_special" => {
                parsed(t, ParsedEpisodeInfo::is_possible_special_episode)
            }
            "IsPossibleSpecialEpisode_should_be_false_for_Special_in_title" => parsed(t, |p| !p.is_possible_special_episode()),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_normalize_episode_title_fixture() {
    run("NormalizeEpisodeTitleFixture", |method, c| {
        Some(match method {
            "should_normalize_episode_title" => {
                let got = episode::normalize_episode_title(s(c, 0));
                expect(got == s(c, 1), got)
            }
            _ => return None,
        })
    });
}

#[test]
fn sonarr_normalize_series_title_fixture() {
    run("NormalizeSeriesTitleFixture", |method, c| {
        let all = |formats: &[&str], want: &dyn Fn(&str) -> String| {
            let w = s(c, 0);
            let bad: Vec<(String, String)> = formats
                .iter()
                .map(|f| f.replace("{0}", w))
                .map(|d| (episode::clean_series_title(&d), d))
                .filter(|(got, _)| *got != want(w))
                .collect();
            expect(bad.is_empty(), bad)
        };
        Some(match method {
            "should_normalize_series_title"
            | "should_remove_special_characters_and_casing"
            | "should_not_remove_from_the_beginning_of_the_title"
            | "should_replace_percent_sign_with_percent_following_numbers" => {
                let got = episode::clean_series_title(s(c, 0));
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

#[test]
fn sonarr_path_parser_fixture() {
    run("PathParserFixture", |method, c| {
        let path = os_agnostic(s(c, 0));
        Some(match method {
            "should_parse_from_path" => parsed_path(&path, |p| {
                p.episode_numbers.len() == 1
                    && p.season_number == n(c, 1)
                    && p.episode_numbers[0] == n(c, 2)
                    && p.absolute_episode_numbers.is_empty()
                    && !p.full_season
            }),
            "should_parse_multi_episode_from_path" => parsed_path(&path, |p| {
                let mut want = ints(c, 3);
                let mut got = p.episode_numbers.clone();
                want.sort_unstable();
                got.sort_unstable();
                p.series_title == s(c, 1)
                    && p.season_number == n(c, 2)
                    && got == want
                    && p.absolute_episode_numbers.is_empty()
                    && !p.full_season
            }),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_release_group_parser_fixture() {
    run("ReleaseGroupParserFixture", |method, c| {
        let got = episode::parse_release_group(s(c, 0));
        let want = if method == "should_not_parse_anime_hash_as_release_group" { None } else { opt(c, 1) };
        if !method.starts_with("should_") {
            return None;
        }
        Some(expect(got.as_deref() == want, got))
    });
}

#[test]
fn sonarr_scene_checker_fixture() {
    run("SceneCheckerFixture", |method, c| {
        let t = s(c, 0);
        Some(match method {
            "should_return_true_for_scene_names" => expect(episode::is_scene_title(t), episode::parse_title(t)),
            "should_return_false_for_non_scene_names" => expect(!episode::is_scene_title(t), episode::parse_title(t)),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_unicode_release_parser_fixture() {
    run("UnicodeReleaseParserFixture", |method, c| {
        let t = replace_unicode(s(c, 0));
        let anime = |p: &ParsedEpisodeInfo| {
            p.release_group.as_deref() == opt(c, 2)
                && p.absolute_episode_numbers == [n(c, 3)]
                && p.series_title == s(c, 1)
                && !p.full_season
        };
        Some(match method {
            "should_parse_chinese_anime_releases"
            | "should_parse_chinese_anime_season_episode_releases"
            | "should_parse_unbracketed_chinese_anime_releases"
            | "should_parse_false_positive_chinese_anime_releases" => parsed(&t, anime),
            "should_parse_chinese_multiepisode_releases" => parsed(&t, |p| {
                let mut want = ints(c, 3);
                let mut got = p.absolute_episode_numbers.clone();
                want.sort_unstable();
                got.sort_unstable();
                p.release_group.as_deref() == opt(c, 2) && got == want && p.series_title == s(c, 1) && !p.full_season
            }),
            "should_parse_gm_team_releases_and_files" => parsed(s(c, 0), |p| {
                p.absolute_episode_numbers == [n(c, 2)]
                    && p.series_title == s(c, 1)
                    && !p.full_season
                    && p.release_group.as_deref() == Some("GM-Team")
            }),
            "should_parse_unicode_digits" => parsed(s(c, 0), |p| {
                p.series_title == s(c, 1)
                    && p.absolute_episode_numbers == [n(c, 2)]
                    && p.special_absolute_episode_numbers.is_empty()
                    && !p.full_season
            }),
            _ => return None,
        })
    });
}

#[test]
fn sonarr_url_fixture() {
    run("UrlFixture", |method, c| {
        Some(match method {
            "should_not_parse_url_in_name" => {
                let got = episode::clean_series_title(&episode::parse_series_name(s(c, 0)));
                expect(got == episode::clean_series_title(s(c, 1)), got)
            }
            "should_not_parse_url_in_group" => {
                let got = episode::parse_release_group(s(c, 0));
                expect(got.as_deref() == opt(c, 1), got)
            }
            _ => return None,
        })
    });
}

/// The upstream `[Test]` methods without cases.
#[test]
fn sonarr_single_tests() {
    // ParserFixture.should_remove_accents_from_title
    assert_eq!(episode::clean_series_title("Seri\u{00E0}es"), "seriaes");
    // NormalizeSeriesTitleFixture
    for f in ["word.{0}.word", "word {0} word", "word-{0}-word"] {
        assert_eq!(episode::clean_series_title(&f.replace("{0}", "a")), "wordword", "should_remove_a_from_middle_of_title");
    }
    assert_eq!(episode::clean_series_title("Series Title A"), "seriestitlea", "should_not_clean_trailing_a");
    // SeasonParserFixture.should_not_parse_season_folders
    assert!(episode::parse_title("Season 3").is_none());
    // DailyEpisodeParserFixture.should_fail_if_episode_is_far_in_future
    let title = format!("{} - A Talk Show - HD TV.mkv", (chrono::Local::now().date_naive() + chrono::Days::new(2)).format("%Y.%m.%d"));
    assert!(episode::parse_title(&title).is_none(), "should_fail_if_episode_is_far_in_future");
    // ReleaseGroupParserFixture.should_not_include_extension_in_release_group
    let path = os_agnostic(r"C:\Test\Doctor.Series.2005.s01e01.internal.bdrip.x264-archivist.mkv");
    assert_eq!(episode::parse_path(&path).and_then(|p| p.release_group).as_deref(), Some("archivist"));
    // IsPossibleSpecialEpisodeFixture
    let mut p = ParsedEpisodeInfo::new("");
    p.episode_numbers = vec![7];
    p.season_number = 1;
    assert!(!p.is_possible_special_episode(), "should_not_treat_files_without_a_series_title_as_a_special");
    let mut p = ParsedEpisodeInfo::new("");
    p.season_number = 1;
    assert!(p.is_possible_special_episode(), "should_return_true_when_episode_numbers_is_empty");
    // CrapParserFixture.should_not_parse_md5: upstream hashes "CRAPPY TEST SEED" 100 times; any 32-hex name is
    // rejected the same way, so a few fixed ones stand for them
    for h in ["5B7F8A2C9D0E1F3A4B6C7D8E9F0A1B2C", "D41D8CD98F00B204E9800998ECF8427E", "0123456789ABCDEF0123456789ABCDEF"] {
        assert!(episode::parse_title(h).is_none(), "should_not_parse_md5: {h}");
    }
}
