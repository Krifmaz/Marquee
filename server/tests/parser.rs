//! The upstream parser tests, run against the Rust ports. The cases are Sonarr's and Radarr's own NUnit
//! `[TestCase]`s, turned into JSON by `scripts/upstream_cases.py` (see `server/tests/upstream/`); each test here
//! checks what the upstream test method checks.

use serde_json::Value;
use server::parser::quality::{tv, DetectionSource, Modifier, MovieQuality, MovieSource, TvQuality};
use server::parser::{quality_movie, quality_tv};
use std::collections::BTreeMap;

/// One upstream fixture: test method → its cases.
type Fixture = BTreeMap<String, Vec<Vec<Value>>>;

/// Reads `server/tests/upstream/<app>/<name>.json`.
fn fixture(app: &str, name: &str) -> Fixture {
    let path = format!("{}/tests/upstream/{app}/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("valid JSON")
}

/// The cases of `method`, which must exist and hold at least one case.
fn cases<'a>(f: &'a Fixture, method: &str) -> &'a [Vec<Value>] {
    let c = f.get(method).unwrap_or_else(|| panic!("the upstream fixture has no {method}"));
    assert!(!c.is_empty(), "{method} has no cases");
    c
}

/// A case's string argument `i`.
fn s(case: &[Value], i: usize) -> &str {
    case[i].as_str().unwrap_or_else(|| panic!("argument {i} of {case:?} is not a string"))
}

/// A case's bool argument `i`, false when the case leaves it out.
fn b(case: &[Value], i: usize) -> bool {
    case.get(i).and_then(Value::as_bool).unwrap_or(false)
}

/// Collects failures so one run shows all of them.
#[derive(Default)]
struct Failures(Vec<String>);

impl Failures {
    /// Records a failure when `ok` is false.
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if !ok {
            self.0.push(what());
        }
    }

    /// Fails the test with every failure, if any.
    fn finish(self, total: usize) {
        eprintln!("{total} upstream cases checked");
        assert!(self.0.is_empty(), "{} of {total} upstream cases failed:\n{}", self.0.len(), self.0.join("\n"));
    }
}

#[test]
fn sonarr_quality_parser_fixture() {
    let f = fixture("sonarr", "QualityParserFixture");
    let expect: &[(&str, TvQuality)] = &[
        ("should_parse_sdtv_quality", tv::SDTV),
        ("should_parse_dvd_quality", tv::DVD),
        ("should_parse_webdl480p_quality", tv::WEBDL480p),
        ("should_parse_bluray480p_quality", tv::Bluray480p),
        ("should_parse_webrip480p_quality", tv::WEBRip480p),
        ("should_parse_bluray576p_quality", tv::Bluray576p),
        ("should_parse_hdtv720p_quality", tv::HDTV720p),
        ("should_parse_hdtv1080p_quality", tv::HDTV1080p),
        ("should_parse_hdtv2160p_quality", tv::HDTV2160p),
        ("should_parse_webdl720p_quality", tv::WEBDL720p),
        ("should_parse_webrip720p_quality", tv::WEBRip720p),
        ("should_parse_webdl1080p_quality", tv::WEBDL1080p),
        ("should_parse_webrip1080p_quality", tv::WEBRip1080p),
        ("should_parse_webdl2160p_quality", tv::WEBDL2160p),
        ("should_parse_webrip2160p_quality", tv::WEBRip2160p),
        ("should_parse_bluray720p_quality", tv::Bluray720p),
        ("should_parse_bluray1080p_quality", tv::Bluray1080p),
        ("should_parse_bluray1080p_remux_quality", tv::Bluray1080pRemux),
        ("should_parse_bluray2160p_quality", tv::Bluray2160p),
        ("should_parse_bluray2160p_remux_quality", tv::Bluray2160pRemux),
        ("should_parse_raw_quality", tv::RAWHD),
        ("quality_parse", tv::Unknown),
    ];
    let mut fail = Failures::default();
    let mut total = 0;
    for (method, quality) in expect {
        for c in cases(&f, method) {
            total += 1;
            let r = quality_tv::parse_quality(s(c, 0));
            let version = if b(c, 1) { 2 } else { 1 };
            fail.check(r.quality == *quality && r.revision.version == version, || {
                format!("{method} {:?}: got {} v{}, want {} v{version}", s(c, 0), r.quality.name, r.revision.version, quality.name)
            });
        }
    }
    // parsing_our_own_quality_enum_name
    for q in [
        tv::SDTV,
        tv::DVD,
        tv::WEBDL480p,
        tv::Bluray480p,
        tv::Bluray576p,
        tv::HDTV720p,
        tv::HDTV1080p,
        tv::HDTV2160p,
        tv::WEBDL720p,
        tv::WEBDL1080p,
        tv::WEBDL2160p,
        tv::Bluray720p,
        tv::Bluray1080p,
        tv::Bluray2160p,
        tv::Bluray1080pRemux,
        tv::Bluray2160pRemux,
    ] {
        total += 1;
        let r = quality_tv::parse_quality(&format!("My series S01E01 [{}]", q.name));
        fail.check(r.quality == q, || format!("own name {}: got {}", q.name, r.quality.name));
    }
    // should_parse_quality_from_other_source
    for (text, q) in [
        ("SD TV", tv::SDTV),
        ("SD DVD", tv::DVD),
        ("480p WEB-DL", tv::WEBDL480p),
        ("HD TV", tv::HDTV720p),
        ("1080p HD TV", tv::HDTV1080p),
        ("2160p HD TV", tv::HDTV2160p),
        ("720p WEB-DL", tv::WEBDL720p),
        ("1080p WEB-DL", tv::WEBDL1080p),
        ("2160p WEB-DL", tv::WEBDL2160p),
        ("720p BluRay", tv::Bluray720p),
        ("1080p BluRay", tv::Bluray1080p),
        ("2160p BluRay", tv::Bluray2160p),
        ("1080p Remux", tv::Bluray1080pRemux),
        ("2160p Remux", tv::Bluray2160pRemux),
    ] {
        for sep in ['-', '.', ' ', '_'] {
            total += 1;
            let title = format!("My series S01E01 {}", text.replace(' ', &sep.to_string()));
            let r = quality_tv::parse_quality(&title);
            fail.check(r.quality == q && r.revision.version == 1, || format!("other source {title:?}: got {}", r.quality.name));
        }
    }
    detection_cases(&f, &mut fail, &mut total, |t| {
        let r = quality_tv::parse_quality(t);
        (r.source_detection, r.resolution_detection)
    });
    for c in cases(&f, "should_be_able_to_parse_repack") {
        total += 1;
        let r = quality_tv::parse_quality(s(c, 0));
        let want = (b(c, 1), c[2].as_i64().unwrap_or(0));
        fail.check((r.revision.is_repack, r.revision.version) == want, || {
            format!("repack {:?}: got {:?}, want {want:?}", s(c, 0), (r.revision.is_repack, r.revision.version))
        });
    }
    for c in cases(&f, "should_be_able_to_parse_anime_version") {
        total += 1;
        let r = quality_tv::parse_quality(s(c, 0));
        fail.check(r.revision.version == c[1].as_i64().unwrap_or(0), || format!("anime version {:?}: got {}", s(c, 0), r.revision.version));
    }
    fail.finish(total);
}

/// The detection-source methods both fixtures share.
fn detection_cases(f: &Fixture, fail: &mut Failures, total: &mut usize, parse: impl Fn(&str) -> (DetectionSource, DetectionSource)) {
    use DetectionSource::*;
    for (method, want) in [
        ("should_parse_full_quality_from_name", (Name, Name)),
        ("should_parse_resolution_from_name", (Unknown, Name)),
        ("should_parse_source_from_name", (Name, Unknown)),
        ("should_parse_quality_from_extension", (Extension, Extension)),
        ("should_parse_resolution_from_name_and_source_from_extension", (Extension, Name)),
    ] {
        for c in cases(f, method) {
            *total += 1;
            let got = parse(s(c, 0));
            fail.check(got == want, || format!("{method} {:?}: got {got:?}, want {want:?}", s(c, 0)));
        }
    }
}

#[test]
fn radarr_quality_parser_fixture() {
    use MovieSource as S;
    let f = fixture("radarr", "QualityParserFixture");
    // method → (source, resolution or 0 for "don't check", modifier or None for "don't check")
    let expect: &[(&str, S, i64, Modifier)] = &[
        ("should_parse_ts", S::Telesync, 720, Modifier::None),
        ("should_parse_cam", S::Cam, 0, Modifier::None),
        ("should_parse_sdtv_quality", S::Tv, 480, Modifier::None),
        ("should_parse_dvd_quality", S::Dvd, 0, Modifier::None),
        ("should_parse_dvdr_quality", S::Dvd, 480, Modifier::Remux),
        ("should_parse_webdl480p_quality", S::Webdl, 480, Modifier::None),
        ("should_parse_webrip480p_quality", S::Webrip, 480, Modifier::None),
        ("should_parse_bluray480p_quality", S::Bluray, 480, Modifier::None),
        ("should_parse_hdtv720p_quality", S::Tv, 720, Modifier::None),
        ("should_parse_hdtv1080p_quality", S::Tv, 1080, Modifier::None),
        ("should_parse_hdtv2160p_quality", S::Tv, 2160, Modifier::None),
        ("should_parse_webdl720p_quality", S::Webdl, 720, Modifier::None),
        ("should_parse_webrip720p_quality", S::Webrip, 720, Modifier::None),
        ("should_parse_webdl1080p_quality", S::Webdl, 1080, Modifier::None),
        ("should_parse_webrip1080p_quality", S::Webrip, 1080, Modifier::None),
        ("should_parse_webdl2160p_quality", S::Webdl, 2160, Modifier::None),
        ("should_parse_webrip2160p_quality", S::Webrip, 2160, Modifier::None),
        ("should_parse_bluray720p_quality", S::Bluray, 720, Modifier::None),
        ("should_parse_bluray1080p_quality", S::Bluray, 1080, Modifier::None),
        ("should_parse_bluray2160p_quality", S::Bluray, 2160, Modifier::None),
        ("should_parse_bluray576p_quality", S::Bluray, 576, Modifier::None),
        ("should_parse_remux720p_as_bluray720p_quality", S::Bluray, 720, Modifier::None),
        ("should_parse_remux1080p_quality", S::Bluray, 1080, Modifier::Remux),
        ("should_parse_remux2160p_quality", S::Bluray, 2160, Modifier::Remux),
        ("should_parse_brdisk_1080p_quality", S::Bluray, 1080, Modifier::BrDisk),
        ("should_parse_raw_quality", S::Tv, 1080, Modifier::RawHd),
        ("quality_parse", S::Unknown, 0, Modifier::None),
    ];
    let mut fail = Failures::default();
    let mut total = 0;
    let verify = |fail: &mut Failures, title: &str, source: S, res: i64, modifier: Modifier, proper: bool, what: &str| {
        let r = quality_movie::parse_quality(title);
        let q: MovieQuality = r.quality;
        let version = if proper { 2 } else { 1 };
        let ok = (res == 0 || q.resolution == res)
            && q.source == source
            && (modifier == Modifier::None || q.modifier == modifier)
            && r.revision.version == version;
        fail.check(ok, || {
            format!(
                "{what} {title:?}: got {} ({:?} {} {:?}) v{}, want {source:?} {res} {modifier:?} v{version}",
                q.name, q.source, q.resolution, q.modifier, r.revision.version
            )
        });
    };
    for (method, source, res, modifier) in expect {
        for c in cases(&f, method) {
            total += 1;
            verify(&mut fail, s(c, 0), *source, *res, *modifier, b(c, 1), method);
        }
    }
    for (text, source, res, modifier) in [
        ("SD TV", S::Tv, 480, Modifier::None),
        ("SD DVD", S::Dvd, 0, Modifier::None),
        ("480p WEB-DL", S::Webdl, 480, Modifier::None),
        ("HD TV", S::Tv, 720, Modifier::None),
        ("1080p HD TV", S::Tv, 1080, Modifier::None),
        ("2160p HD TV", S::Tv, 2160, Modifier::None),
        ("720p WEB-DL", S::Webdl, 720, Modifier::None),
        ("1080p WEB-DL", S::Webdl, 1080, Modifier::None),
        ("2160p WEB-DL", S::Webdl, 2160, Modifier::None),
        ("720p BluRay", S::Bluray, 720, Modifier::None),
        ("1080p BluRay", S::Bluray, 1080, Modifier::None),
        ("2160p BluRay", S::Bluray, 2160, Modifier::None),
        ("1080p Remux", S::Bluray, 1080, Modifier::Remux),
        ("2160p Remux", S::Bluray, 2160, Modifier::Remux),
    ] {
        for sep in ['-', '.', ' ', '_'] {
            total += 1;
            let title = format!("My series S01E01 {}", text.replace(' ', &sep.to_string()));
            verify(&mut fail, &title, source, res, modifier, false, "other source");
        }
    }
    detection_cases(&f, &mut fail, &mut total, |t| {
        let r = quality_movie::parse_quality(t);
        (r.source_detection, r.resolution_detection)
    });
    for c in cases(&f, "should_be_able_to_parse_repack") {
        total += 1;
        let r = quality_movie::parse_quality(s(c, 0));
        let want = (b(c, 1), c[2].as_i64().unwrap_or(0));
        fail.check((r.revision.is_repack, r.revision.version) == want, || {
            format!("repack {:?}: got {:?}, want {want:?}", s(c, 0), (r.revision.is_repack, r.revision.version))
        });
    }
    fail.finish(total);
}
