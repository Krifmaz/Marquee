//! Radarr's `QualityParser`: the quality a movie release name says. Port of
//! `src/NzbDrone.Core/Parser/QualityParser.cs` at the Radarr commit in `UPSTREAM.lock`; the expressions are the
//! upstream ones, unchanged.

use super::net_regex::{NetRegex, I, IX, NONE};
use super::quality::{movie, path_extension, DetectionSource as D, Modifier, MovieQuality, MovieSource, QualityModel};
use super::quality_tv::Resolution;
use std::sync::LazyLock;

/// The source words.
static SOURCE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\b(?:
                                                            (?<bluray>M?Blu[-_. ]?Ray|HD[-_. ]?DVD|BD(?!$)|UHD2?BD|BDISO|BDMux|BD25|BD50|BR[-_. ]?DISK)|
                                                            (?<webdl>WEB[-_. ]?DL(?:mux)?|AmazonHD|AmazonSD|iTunesHD|MaxdomeHD|NetflixU?HD|WebHD|HBOMaxHD|DisneyHD|[. ]WEB[. ](?:[xh][ .]?26[45]|AVC|HEVC|DDP?5[. ]1)|[. ](?-i:WEB)$|(?:\d{3,4}0p)[-. ](?:Hybrid[-_. ]?)?WEB[-. ]|[-. ]WEB[-. ]\d{3,4}0p|\b\s\/\sWEB\s\/\s\b|(?:AMZN|NF|DP)[. -]WEB[. -](?!Rip))|
                                                            (?<webrip>WebRip|Web-Rip|WEBMux)|
                                                            (?<hdtv>HDTV)|
                                                            (?<bdrip>BDRip|BDLight|HD[-_. ]?DVDRip|UHDBDRip)|
                                                            (?<brrip>BRRip)|
                                                            (?<dvdr>\d?x?M?DVD-?[R59])|
                                                            (?<dvd>DVD(?!-R)|DVDRip|xvidvd)|
                                                            (?<dsr>WS[-_. ]DSR|DSR)|
                                                            (?<regional>R[0-9]{1}|REGIONAL)|
                                                            (?<scr>SCR|SCREENER|DVDSCR|DVDSCREENER)|
                                                            (?<ts>TS[-_. ]|TELESYNCH?|HD-TS|HDTS|PDVD|TSRip|HDTSRip)|
                                                            (?<tc>TC|TELECINE|HD-TC|HDTC)|
                                                            (?<cam>CAMRIP|(?:NEW)?CAM|HD-?CAM(?:Rip)?|HQCAM)|
                                                            (?<wp>WORKPRINT|WP)|
                                                            (?<pdtv>PDTV)|
                                                            (?<sdtv>SDTV)|
                                                            (?<tvrip>TVRip)
                                                            )(?:\b|$|[ .])",
        IX,
    )
});
/// Raw-HD.
static RAW_HD: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<rawhd>RawHD|Raw[-_. ]HD)\b", I));
/// MPEG-2 (a raw HDTV stream).
static MPEG2: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<mpeg2>MPEG[-_. ]?2)\b", NONE));
/// A whole Blu-ray disc.
static BR_DISK: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"^(?!.*\b((?<!HD[._ -]|HD)DVD|BDRip|720p|MKV|XviD|WMV|d3g|(BD)?REMUX|^(?=.*1080p)(?=.*HEVC)|[xh][-_. ]?26[45]|German.*[DM]L|((?<=\d{4}).*German.*([DM]L)?)(?=.*\b(AVC|HEVC|VC[-_. ]?1|MVC|MPEG[-_. ]?2)\b))\b)(((?=.*\b(Blu[-_. ]?ray|BD|HD[-_. ]?DVD)\b)(?=.*\b(AVC|HEVC|VC[-_. ]?1|MVC|MPEG[-_. ]?2|BDMV|ISO)\b))|^((?=.*\b(((?=.*\b((.*_)?COMPLETE.*|Dis[ck])\b)(?=.*(Blu[-_. ]?ray|HD[-_. ]?DVD)))|3D[-_. ]?BD|BR[-_. ]?DISK|Full[-_. ]?Blu[-_. ]?ray|^((?=.*((BD|UHD)[-_. ]?(25|50|66|100|ISO)))))))).*",
        I,
    )
});
/// PROPER.
static PROPER: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<proper>proper)\b", I));
/// REPACK and RERIP.
static REPACK: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<repack>repack\d?|rerip\d?)\b", I));
/// A version number (`v2`, `REPACK2`).
static VERSION: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"\d[-._ ]?v(?<version>\d)[-._ ]|\[v(?<version>\d)\]|repack(?<version>\d)|rerip(?<version>\d)", I));
/// REAL (case matters).
static REAL: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<real>REAL)\b", NONE));
/// The resolution.
static RESOLUTION: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\b(?:(?<R360p>360p)|(?<R480p>480p|480i|640x480|848x480)|(?<R540p>540p)|(?<R576p>576p)|(?<R720p>720p|1280x720|960p)|(?<R1080p>1080p|1920x1080|1440p|FHD|1080i|4kto1080p)|(?<R2160p>2160p|3840x2160|4k[-_. ](?:UHD|HEVC|BD|H\.?265)|(?:UHD|HEVC|BD|H\.?265)[-_. ]4k))\b",
        I,
    )
});
/// UHD or `[4K]` with no resolution.
static ALTERNATIVE_RESOLUTION: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<R2160p>UHD)\b|(?<R2160p>\[4K\])", I));
/// The codec.
static CODEC: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"\b(?:(?<x264>x264)|(?<h264>h264)|(?<xvidhd>XvidHD)|(?<xvid>X-?vid)|(?<divx>divx))\b", I));
/// `HD TV` and `SD TV`.
static OTHER_SOURCE: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"(?<hdtv>HD[-_. ]TV)|(?<sdtv>SD[-_. ]TV)", I));
/// Anime Blu-ray (`BD1080`, `[BD]`).
static ANIME_BLURAY: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"bd(?:720|1080|2160)|(?<=[-_. (\[])bd(?=[-_. )\]])", I));
/// Anime WEB (`[WEB]`).
static ANIME_WEBDL: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\[WEB\]|[\[\(]WEB[ .]", I));
/// `HR.WS` (high-resolution PDTV).
static HIGH_DEF_PDTV: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"hr[-_. ]ws", I));
/// Remux.
static REMUX: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(r"(?:[_. \[]|\d{4}p-|\bHybrid-)(?<remux>(?:(BD|UHD)[-_. ]?)?Remux)\b|(?<remux>(?:(BD|UHD)[-_. ]?)?Remux[_. ]\d{4}p)", I)
});
/// German Blu-ray releases that are remuxes without saying so.
static GERMAN_REMUX: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"((?<=\d{4}).*German.*([DM]L)?)(?=.*\b(AVC|HEVC|VC[_. -]?1|MVC|MPEG[_. -]?2))(?=.*Blu-?ray)", I));

/// `string.ContainsIgnoreCase`.
fn has_i(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// `ParseQuality`: the quality `name` says, falling back to its file extension.
pub fn parse_quality(name: &str) -> QualityModel<MovieQuality> {
    if name.trim().is_empty() {
        return QualityModel::new(movie::Unknown);
    }
    let name = name.trim();
    let mut result = parse_quality_name(name);
    if result.quality == movie::Unknown && !name.contains('\0') {
        result.quality = MovieQuality::for_extension(path_extension(name));
        result.source_detection = D::Extension;
        result.resolution_detection = D::Extension;
        result.modifier_detection = D::Extension;
    }
    result
}

/// `ParseQualityName`: the quality `name` says, without looking at the extension (except to learn a source).
pub fn parse_quality_name(name: &str) -> QualityModel<MovieQuality> {
    let normalized = name.replace('_', " ");
    let normalized = normalized.trim();
    let mut result = parse_quality_modifiers(name, normalized);
    use Resolution as R;

    let source_match = SOURCE.captures_iter(normalized).pop();
    let resolution = parse_resolution(normalized);
    let codec = CODEC.captures(normalized);
    let codec_ok = |g: &str| codec.as_ref().is_some_and(|c| c.ok(g));
    let remux = REMUX.is_match(normalized) || GERMAN_REMUX.is_match(normalized);
    let br_disk = BR_DISK.is_match(normalized);

    if RAW_HD.is_match(normalized) && !br_disk {
        result.source_detection = D::Name;
        result.resolution_detection = D::Name;
        result.quality = movie::RAWHD;
        return result;
    }

    if resolution != R::Unknown {
        result.resolution_detection = D::Name;
    }

    if let Some(m) = &source_match {
        result.source_detection = D::Name;

        if m.ok("bluray") {
            result.quality = if br_disk {
                movie::BRDISK
            } else if codec_ok("xvid") || codec_ok("divx") {
                movie::Bluray480p
            } else if resolution == R::R2160p {
                if remux {
                    movie::Remux2160p
                } else {
                    movie::Bluray2160p
                }
            } else if resolution == R::R1080p {
                if remux {
                    movie::Remux1080p
                } else {
                    movie::Bluray1080p
                }
            } else if resolution == R::R720p {
                movie::Bluray720p
            } else if resolution == R::R576p {
                movie::Bluray576p
            } else if matches!(resolution, R::R360p | R::R480p | R::R540p) {
                movie::Bluray480p
            } else if remux && resolution != R::R720p {
                // a remux without a resolution is 1080p, not 720p
                movie::Remux1080p
            } else {
                movie::Bluray720p
            };
            return result;
        }

        if m.ok("webdl") {
            result.quality = match resolution {
                R::R2160p => movie::WEBDL2160p,
                R::R1080p => movie::WEBDL1080p,
                R::R720p => movie::WEBDL720p,
                _ if name.contains("[WEBDL]") => movie::WEBDL720p,
                _ => movie::WEBDL480p,
            };
            return result;
        }

        if m.ok("webrip") {
            result.quality = match resolution {
                R::R2160p => movie::WEBRip2160p,
                R::R1080p => movie::WEBRip1080p,
                R::R720p => movie::WEBRip720p,
                _ => movie::WEBRip480p,
            };
            return result;
        }

        if m.ok("scr") {
            result.quality = movie::DVDSCR;
            return result;
        }
        if m.ok("cam") {
            result.quality = movie::CAM;
            return result;
        }
        if m.ok("ts") {
            result.quality = movie::TELESYNC;
            result.quality.resolution = resolution_lines(resolution);
            return result;
        }
        if m.ok("tc") {
            result.quality = movie::TELECINE;
            return result;
        }
        if m.ok("wp") {
            result.quality = movie::WORKPRINT;
            return result;
        }
        if m.ok("regional") {
            result.quality = movie::REGIONAL;
            return result;
        }

        if m.ok("hdtv") {
            result.quality = if MPEG2.is_match(normalized) {
                movie::RAWHD
            } else {
                match resolution {
                    R::R2160p => movie::HDTV2160p,
                    R::R1080p => movie::HDTV1080p,
                    R::R720p => movie::HDTV720p,
                    _ if name.contains("[HDTV]") => movie::HDTV720p,
                    _ => movie::SDTV,
                }
            };
            return result;
        }

        if m.ok("bdrip") || m.ok("brrip") {
            result.quality = match resolution {
                R::R720p => movie::Bluray720p,
                R::R1080p => movie::Bluray1080p,
                R::R2160p => movie::Bluray2160p,
                R::R576p => movie::Bluray576p,
                _ => movie::Bluray480p,
            };
            return result;
        }

        if m.ok("dvdr") {
            result.quality = movie::DVDR;
            return result;
        }
        if m.ok("dvd") {
            result.quality = movie::DVD;
            return result;
        }

        if m.ok("pdtv") || m.ok("sdtv") || m.ok("dsr") || m.ok("tvrip") {
            if resolution == R::R1080p || has_i(normalized, "1080p") {
                result.quality = movie::HDTV1080p;
                return result;
            }
            if resolution == R::R720p || has_i(normalized, "720p") {
                result.quality = movie::HDTV720p;
                return result;
            }
            if HIGH_DEF_PDTV.is_match(normalized) {
                result.resolution_detection = D::Name;
                result.quality = movie::HDTV720p;
                return result;
            }
            result.quality = movie::SDTV;
            return result;
        }
    }

    if source_match.is_none() && remux && resolution != R::Unknown {
        result.source_detection = D::Unknown;
        let q = match resolution {
            R::R480p => Some(movie::Bluray480p),
            R::R720p => Some(movie::Bluray720p),
            R::R2160p => Some(movie::Remux2160p),
            R::R1080p => Some(movie::Remux1080p),
            _ => None,
        };
        if let Some(q) = q {
            result.quality = q;
            return result;
        }
    }

    // anime Blu-ray
    if ANIME_BLURAY.is_match(normalized) {
        result.source_detection = D::Name;
        if matches!(resolution, R::R360p | R::R480p | R::R540p | R::R576p) || has_i(normalized, "480p") {
            result.resolution_detection = D::Name;
            result.quality = movie::DVD;
            return result;
        }
        if resolution == R::R1080p || has_i(normalized, "1080p") {
            result.resolution_detection = D::Name;
            result.quality = if remux { movie::Remux1080p } else { movie::Bluray1080p };
            return result;
        }
        if resolution == R::R2160p || has_i(normalized, "2160p") {
            result.resolution_detection = D::Name;
            result.quality = if remux { movie::Remux2160p } else { movie::Bluray2160p };
            return result;
        }
        result.quality = if remux && resolution != R::R720p { movie::Remux1080p } else { movie::Bluray720p };
        return result;
    }

    if ANIME_WEBDL.is_match(normalized) {
        result.source_detection = D::Name;
        if matches!(resolution, R::R360p | R::R480p | R::R540p | R::R576p) || has_i(normalized, "480p") {
            result.resolution_detection = D::Name;
            result.quality = movie::WEBDL480p;
            return result;
        }
        if resolution == R::R1080p || has_i(normalized, "1080p") {
            result.resolution_detection = D::Name;
            result.quality = movie::WEBDL1080p;
            return result;
        }
        if resolution == R::R2160p || has_i(normalized, "2160p") {
            result.resolution_detection = D::Name;
            result.quality = movie::WEBDL2160p;
            return result;
        }
        result.quality = movie::WEBDL720p;
        return result;
    }

    if resolution != R::Unknown {
        let mut source = MovieSource::Unknown;
        let mut modifier = Modifier::None;
        if remux {
            result.source_detection = D::Name;
            source = MovieSource::Bluray;
            modifier = Modifier::Remux;
        } else {
            let ext = name.rfind('.').filter(|i| i + 1 < name.len()).map_or("", |i| &name[i..]);
            let q = MovieQuality::for_extension(ext);
            if q != movie::Unknown {
                result.source_detection = D::Extension;
                source = q.source;
            }
        }
        let pick = |fallback: MovieQuality, res: i64| {
            if source == MovieSource::Unknown {
                fallback
            } else {
                MovieQuality::find_by_source_and_resolution(source, res, modifier)
            }
        };
        let q = match resolution {
            R::R2160p => Some(pick(movie::HDTV2160p, 2160)),
            R::R1080p => Some(pick(movie::HDTV1080p, 1080)),
            R::R720p => Some(pick(movie::HDTV720p, 720)),
            R::R360p | R::R480p | R::R540p | R::R576p => Some(pick(movie::SDTV, 480)),
            R::Unknown => None,
        };
        if let Some(q) = q {
            result.resolution_detection = D::Name;
            result.quality = q;
            return result;
        }
    }

    if codec_ok("x264") {
        result.quality = movie::SDTV;
        return result;
    }

    if has_i(normalized, "848x480") {
        result.resolution_detection = D::Name;
        if normalized.contains("dvd") {
            result.source_detection = D::Name;
            result.quality = movie::DVD;
        } else if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = movie::Bluray480p;
        } else {
            result.quality = movie::SDTV;
        }
        return result;
    }

    if has_i(normalized, "1280x720") {
        result.resolution_detection = D::Name;
        if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = movie::Bluray720p;
        } else {
            result.quality = movie::HDTV720p;
        }
        return result;
    }

    if has_i(normalized, "1920x1080") {
        result.resolution_detection = D::Name;
        if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = movie::Bluray1080p;
        } else {
            result.quality = movie::HDTV1080p;
        }
        return result;
    }

    for (word, q) in [("bluray720p", movie::Bluray720p), ("bluray1080p", movie::Bluray1080p), ("bluray2160p", movie::Bluray2160p)] {
        if has_i(normalized, word) {
            result.source_detection = D::Name;
            result.resolution_detection = D::Name;
            result.quality = q;
            return result;
        }
    }

    let other = other_source_match(normalized);
    if other != movie::Unknown {
        result.source_detection = D::Name;
        result.quality = other;
    }
    result
}

/// The resolution's lines, 0 when not known.
fn resolution_lines(r: Resolution) -> i64 {
    match r {
        Resolution::R360p => 360,
        Resolution::R480p => 480,
        Resolution::R540p => 540,
        Resolution::R576p => 576,
        Resolution::R720p => 720,
        Resolution::R1080p => 1080,
        Resolution::R2160p => 2160,
        Resolution::Unknown => 0,
    }
}

/// `ParseResolution` (Radarr's expression: `H.?265` where Sonarr has `H265`).
pub fn parse_resolution(name: &str) -> Resolution {
    let m = RESOLUTION.captures(name);
    let alt = ALTERNATIVE_RESOLUTION.captures(name);
    let ok = |g: &str| m.as_ref().is_some_and(|c| c.ok(g));
    if m.is_none() && alt.is_none() {
        return Resolution::Unknown;
    }
    if ok("R360p") {
        Resolution::R360p
    } else if ok("R480p") {
        Resolution::R480p
    } else if ok("R540p") {
        Resolution::R540p
    } else if ok("R576p") {
        Resolution::R576p
    } else if ok("R720p") {
        Resolution::R720p
    } else if ok("R1080p") {
        Resolution::R1080p
    } else if ok("R2160p") || alt.as_ref().is_some_and(|c| c.ok("R2160p")) {
        Resolution::R2160p
    } else {
        Resolution::Unknown
    }
}

/// `OtherSourceMatch`: `SD TV` and `HD TV`.
fn other_source_match(name: &str) -> MovieQuality {
    match OTHER_SOURCE.captures(name) {
        Some(m) if m.ok("sdtv") => movie::SDTV,
        Some(m) if m.ok("hdtv") => movie::HDTV720p,
        _ => movie::Unknown,
    }
}

/// `ParseQualityModifiers`: proper, repack, version and REAL.
fn parse_quality_modifiers(name: &str, normalized: &str) -> QualityModel<MovieQuality> {
    let mut result = QualityModel::new(movie::Unknown);
    let version: Option<i64> = VERSION.captures(normalized).map(|c| c.str("version").parse().unwrap_or(0));
    if let Some(v) = version {
        result.revision.version = v;
        result.revision_detection = D::Name;
    }
    if PROPER.is_match(normalized) {
        result.revision.version = version.map_or(2, |v| v + 1);
        result.revision_detection = D::Name;
    }
    if REPACK.is_match(normalized) {
        result.revision.version = version.map_or(2, |v| v + 1);
        result.revision.is_repack = true;
        result.revision_detection = D::Name;
    }
    let real = REAL.count(name) as i64;
    if real > 0 {
        result.revision.real = real;
        result.revision_detection = D::Name;
    }
    result
}
