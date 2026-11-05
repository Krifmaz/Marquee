//! Sonarr's `QualityParser`: the quality a TV release name says. Port of
//! `src/NzbDrone.Core/Parser/QualityParser.cs` at the Sonarr commit in `UPSTREAM.lock`; the expressions are the
//! upstream ones, unchanged.

use super::net_regex::{NetRegex, I, IX, NONE};
use super::quality::{path_extension, tv, DetectionSource as D, QualityModel, TvQuality, TvSource};
use std::sync::LazyLock;

/// A release's resolution (`Resolution`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// 360p
    R360p,
    /// 480p
    R480p,
    /// 540p
    R540p,
    /// 576p
    R576p,
    /// 720p
    R720p,
    /// 1080p
    R1080p,
    /// 2160p
    R2160p,
    /// not known
    Unknown,
}

/// The source words.
static SOURCE: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\b(?:
                                                                (?<bluray>BluRay|Blu-Ray|HD-?DVD|BDMux|BD(?!$))|
                                                                (?<webdl>WEB[-_. ]DL(?:mux)?|WEBDL|AmazonHD|AmazonSD|iTunesHD|MaxdomeHD|NetflixU?HD|WebHD|HBOMaxHD|DisneyHD|[. ]WEB[. ](?:[xh][ .]?26[45]|AVC|HEVC|DDP?5[. ]1)|[. ](?-i:WEB)$|(?:720|1080|2160)p[-. ]WEB[-. ]|[-. ]WEB[-. ](?:720|1080|2160)p|\b\s\/\sWEB\s\/\s\b|(?:AMZN|NF|DP)[. -]WEB[. -](?!Rip))|
                                                                (?<webrip>WebRip|Web-Rip|WEBMux)|
                                                                (?<hdtv>HDTV)|
                                                                (?<bdrip>BDRip|BDLight)|
                                                                (?<brrip>BRRip)|
                                                                (?<dvd>DVD|DVDRip|NTSC|PAL|xvidvd)|
                                                                (?<dsr>WS[-_. ]DSR|DSR)|
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
/// PROPER.
static PROPER: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<proper>proper)\b", I));
/// REPACK and RERIP.
static REPACK: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<repack>repack\d?|rerip\d?)\b", I));
/// A version number (`v2`, `REPACK2`).
static VERSION: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\d[-._ ]?v(?<version>\d)[-._ ]|\[v(?<version>\d)\]|repack(?<version>\d)|rerip(?<version>\d)|(?:480|576|720|1080|2160)p[._ ]v(?<version>\d)",
        I,
    )
});
/// REAL (case matters).
static REAL: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<real>REAL)\b", NONE));
/// The resolution.
static RESOLUTION: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\b(?:(?<R360p>360p)|(?<R480p>480p|480i|640x480|848x480)|(?<R540p>540p)|(?<R576p>576p)|(?<R720p>720p|1280x720|960p)|(?<R1080p>1080p|1920x1080|1440p|FHD|1080i|4kto1080p)|(?<R2160p>2160p|3840x2160|4k[-_. ](?:UHD|HEVC|BD|H265)|(?:UHD|HEVC|BD|H265)[-_. ]4k))\b",
        I,
    )
});
/// UHD or `[4K]` with no resolution.
static ALTERNATIVE_RESOLUTION: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\b(?<R2160p>UHD)\b|(?<R2160p>\[4K\])", I));
/// The codec.
static CODEC: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"\b(?:(?<x264>x264)|(?<h264>h264)|(?<xvidhd>XvidHD)|(?<xvid>Xvid)|(?<divx>divx))\b", I));
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
    NetRegex::new(r"(?:[_. ]|\d{4}p-|\bHybrid-)(?<remux>(?:(BD|UHD)[-_. ]?)?Remux)\b|(?<remux>(?:(BD|UHD)[-_. ]?)?Remux[_. ]\d{4}p)", I)
});

/// `string.ContainsIgnoreCase`.
fn has_i(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// `ParseQuality`: the quality `name` says, falling back to its file extension.
pub fn parse_quality(name: &str) -> QualityModel<TvQuality> {
    if name.trim().is_empty() {
        return QualityModel::new(tv::Unknown);
    }
    let name = name.trim();
    let mut result = parse_quality_name(name);
    if result.quality == tv::Unknown && !name.contains('\0') {
        result.quality = TvQuality::for_extension(path_extension(name));
        result.source_detection = D::Extension;
        result.resolution_detection = D::Extension;
    }
    result
}

/// `ParseQualityName`: the quality `name` says, without looking at the extension (except to learn a source).
pub fn parse_quality_name(name: &str) -> QualityModel<TvQuality> {
    let normalized = name.replace('_', " ");
    let normalized = normalized.trim();
    let mut result = parse_quality_modifiers(name, normalized);

    if RAW_HD.is_match(normalized) {
        result.source_detection = D::Name;
        result.resolution_detection = D::Name;
        result.quality = tv::RAWHD;
        return result;
    }

    let source_match = SOURCE.captures_iter(normalized).pop();
    let resolution = parse_resolution(normalized);
    let codec = CODEC.captures(normalized);
    let codec_ok = |g: &str| codec.as_ref().is_some_and(|c| c.ok(g));
    let remux = REMUX.is_match(normalized);
    use Resolution as R;

    if resolution != R::Unknown {
        result.resolution_detection = D::Name;
    }

    if let Some(m) = &source_match {
        result.source_detection = D::Name;

        if m.ok("bluray") {
            let q = if codec_ok("xvid") || codec_ok("divx") {
                tv::Bluray480p
            } else if resolution == R::R2160p {
                if remux {
                    tv::Bluray2160pRemux
                } else {
                    tv::Bluray2160p
                }
            } else if resolution == R::R1080p {
                if remux {
                    tv::Bluray1080pRemux
                } else {
                    tv::Bluray1080p
                }
            } else if resolution == R::R576p {
                tv::Bluray576p
            } else if matches!(resolution, R::R360p | R::R480p | R::R540p) {
                tv::Bluray480p
            } else if remux && resolution != R::R720p {
                // a remux without a resolution is 1080p, not 720p; a 720p remux is 720p Blu-ray
                tv::Bluray1080pRemux
            } else {
                tv::Bluray720p
            };
            result.quality = q;
            return result;
        }

        if m.ok("webdl") {
            result.quality = match resolution {
                R::R2160p => tv::WEBDL2160p,
                R::R1080p => tv::WEBDL1080p,
                R::R720p => tv::WEBDL720p,
                _ if name.contains("[WEBDL]") => tv::WEBDL720p,
                _ => tv::WEBDL480p,
            };
            return result;
        }

        if m.ok("webrip") {
            result.quality = match resolution {
                R::R2160p => tv::WEBRip2160p,
                R::R1080p => tv::WEBRip1080p,
                R::R720p => tv::WEBRip720p,
                _ => tv::WEBRip480p,
            };
            return result;
        }

        if m.ok("hdtv") {
            result.quality = if MPEG2.is_match(normalized) {
                tv::RAWHD
            } else {
                match resolution {
                    R::R2160p => tv::HDTV2160p,
                    R::R1080p => tv::HDTV1080p,
                    R::R720p => tv::HDTV720p,
                    _ if name.contains("[HDTV]") => tv::HDTV720p,
                    _ => tv::SDTV,
                }
            };
            return result;
        }

        if m.ok("bdrip") || m.ok("brrip") {
            result.quality = match resolution {
                R::R720p => tv::Bluray720p,
                R::R1080p => tv::Bluray1080p,
                R::R2160p => tv::Bluray2160p,
                _ => tv::Bluray480p,
            };
            return result;
        }

        if m.ok("dvd") {
            result.quality = tv::DVD;
            return result;
        }

        if m.ok("pdtv") || m.ok("sdtv") || m.ok("dsr") || m.ok("tvrip") {
            if resolution == R::R1080p || has_i(normalized, "1080p") {
                result.quality = tv::HDTV1080p;
                return result;
            }
            if resolution == R::R720p || has_i(normalized, "720p") {
                result.quality = tv::HDTV720p;
                return result;
            }
            if HIGH_DEF_PDTV.is_match(normalized) {
                result.resolution_detection = D::Name;
                result.quality = tv::HDTV720p;
                return result;
            }
            result.quality = tv::SDTV;
            return result;
        }
    }

    if source_match.is_none() && remux && resolution != R::Unknown {
        result.source_detection = D::Unknown;
        let q = match resolution {
            R::R480p => Some(tv::Bluray480p),
            R::R720p => Some(tv::Bluray720p),
            R::R2160p => Some(tv::Bluray2160pRemux),
            R::R1080p => Some(tv::Bluray1080pRemux),
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
            result.quality = tv::DVD;
            return result;
        }
        if resolution == R::R1080p || has_i(normalized, "1080p") {
            result.resolution_detection = D::Name;
            result.quality = if remux { tv::Bluray1080pRemux } else { tv::Bluray1080p };
            return result;
        }
        if resolution == R::R2160p || has_i(normalized, "2160p") {
            result.resolution_detection = D::Name;
            result.quality = if remux { tv::Bluray2160pRemux } else { tv::Bluray2160p };
            return result;
        }
        result.quality = if remux && resolution != R::R720p { tv::Bluray1080pRemux } else { tv::Bluray720p };
        return result;
    }

    if ANIME_WEBDL.is_match(normalized) {
        result.source_detection = D::Name;
        if matches!(resolution, R::R360p | R::R480p | R::R540p | R::R576p) || has_i(normalized, "480p") {
            result.resolution_detection = D::Name;
            result.quality = tv::WEBDL480p;
            return result;
        }
        if resolution == R::R1080p || has_i(normalized, "1080p") {
            result.resolution_detection = D::Name;
            result.quality = tv::WEBDL1080p;
            return result;
        }
        if resolution == R::R2160p || has_i(normalized, "2160p") {
            result.resolution_detection = D::Name;
            result.quality = tv::WEBDL2160p;
            return result;
        }
        result.quality = tv::WEBDL720p;
        return result;
    }

    if resolution != R::Unknown {
        let mut source = TvSource::Unknown;
        if remux {
            result.source_detection = D::Name;
            source = TvSource::BlurayRaw;
        } else {
            let ext = name.rfind('.').filter(|i| i + 1 < name.len()).map_or("", |i| &name[i..]);
            let q = TvQuality::for_extension(ext);
            if q != tv::Unknown {
                result.source_detection = D::Extension;
                source = q.source;
            }
        }
        let pick = |fallback: TvQuality, res: i64| {
            if source == TvSource::Unknown {
                fallback
            } else {
                TvQuality::find_by_source_and_resolution(source, res)
            }
        };
        let q = match resolution {
            R::R2160p => Some(pick(tv::HDTV2160p, 2160)),
            R::R1080p => Some(pick(tv::HDTV1080p, 1080)),
            R::R720p => Some(pick(tv::HDTV720p, 720)),
            R::R360p | R::R480p | R::R540p | R::R576p => Some(pick(tv::SDTV, 480)),
            R::Unknown => None,
        };
        if let Some(q) = q {
            result.resolution_detection = D::Name;
            result.quality = q;
            return result;
        }
    }

    if codec_ok("x264") {
        result.quality = tv::SDTV;
        return result;
    }

    if normalized.contains("848x480") {
        result.resolution_detection = D::Name;
        if normalized.contains("dvd") {
            result.source_detection = D::Name;
            result.quality = tv::DVD;
        } else if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = tv::Bluray480p;
        } else {
            result.quality = tv::SDTV;
        }
        return result;
    }

    if has_i(normalized, "1280x720") {
        result.resolution_detection = D::Name;
        if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = tv::Bluray720p;
        } else {
            result.quality = tv::HDTV720p;
        }
        return result;
    }

    if has_i(normalized, "1920x1080") {
        result.resolution_detection = D::Name;
        if has_i(normalized, "bluray") {
            result.source_detection = D::Name;
            result.quality = tv::Bluray1080p;
        } else {
            result.quality = tv::HDTV1080p;
        }
        return result;
    }

    for (word, q) in [("bluray720p", tv::Bluray720p), ("bluray1080p", tv::Bluray1080p), ("bluray2160p", tv::Bluray2160p)] {
        if has_i(normalized, word) {
            result.source_detection = D::Name;
            result.resolution_detection = D::Name;
            result.quality = q;
            return result;
        }
    }

    let other = other_source_match(normalized);
    if other != tv::Unknown {
        result.source_detection = D::Name;
        result.quality = other;
    }
    result
}

/// `ParseResolution`.
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
fn other_source_match(name: &str) -> TvQuality {
    match OTHER_SOURCE.captures(name) {
        Some(m) if m.ok("sdtv") => tv::SDTV,
        Some(m) if m.ok("hdtv") => tv::HDTV720p,
        _ => tv::Unknown,
    }
}

/// `ParseQualityModifiers`: proper, repack, version and REAL.
fn parse_quality_modifiers(name: &str, normalized: &str) -> QualityModel<TvQuality> {
    let mut result = QualityModel::new(tv::Unknown);
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
