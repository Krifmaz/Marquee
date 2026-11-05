//! The qualities Sonarr and Radarr know, port of `NzbDrone.Core.Qualities` (`Quality`, `QualitySource`,
//! `Modifier`, `Revision`, `QualityModel`, `QualityFinder`) and `MediaFiles/MediaFileExtensions`.
//!
//! Sonarr and Radarr share most quality ids and names but not all (Radarr has CAM, BR-DISK, `Remux-1080p` = 30
//! where Sonarr has `Bluray-1080p Remux` = 20, …), and order their sources differently, so each has its own list.

/// Where a TV release came from, in Sonarr's order (`QualitySource`); the order matters for
/// [`TvQuality::find_by_source_and_resolution`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TvSource {
    /// not known
    Unknown,
    /// a TV capture
    Television,
    /// an untouched TV stream (Raw-HD)
    TelevisionRaw,
    /// a streaming service's own file (WEB-DL)
    Web,
    /// a capture of a stream (WEBRip)
    WebRip,
    /// a DVD
    Dvd,
    /// a Blu-ray encode
    Bluray,
    /// an untouched Blu-ray (remux)
    BlurayRaw,
}

/// Where a movie release came from, in Radarr's order (`QualitySource`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MovieSource {
    /// not known
    Unknown,
    /// filmed in a cinema
    Cam,
    /// filmed in a cinema with the sound taken directly
    Telesync,
    /// copied from film
    Telecine,
    /// an unfinished cut
    Workprint,
    /// a DVD
    Dvd,
    /// a TV capture
    Tv,
    /// a streaming service's own file
    Webdl,
    /// a capture of a stream
    Webrip,
    /// a Blu-ray
    Bluray,
}

/// What sets a movie quality apart beyond source and resolution (Radarr's `Modifier`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Modifier {
    /// nothing
    None,
    /// a regional DVD release
    Regional,
    /// a screener
    Screener,
    /// an untouched TV stream
    RawHd,
    /// a whole Blu-ray disc
    BrDisk,
    /// an untouched copy of the disc's video
    Remux,
}

/// One of Sonarr's qualities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TvQuality {
    /// Sonarr's id
    pub id: i64,
    /// Sonarr's name
    pub name: &'static str,
    /// where it comes from
    pub source: TvSource,
    /// its resolution in lines, 0 when not known
    pub resolution: i64,
}

/// One of Radarr's qualities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MovieQuality {
    /// Radarr's id
    pub id: i64,
    /// Radarr's name
    pub name: &'static str,
    /// where it comes from
    pub source: MovieSource,
    /// its resolution in lines, 0 when not known
    pub resolution: i64,
    /// what sets it apart
    pub modifier: Modifier,
}

/// Sonarr's qualities by name, as `Quality.<Name>` spells them upstream.
#[allow(non_upper_case_globals, missing_docs)]
pub mod tv {
    use super::{TvQuality as Q, TvSource as S};
    pub const Unknown: Q = Q { id: 0, name: "Unknown", source: S::Unknown, resolution: 0 };
    pub const SDTV: Q = Q { id: 1, name: "SDTV", source: S::Television, resolution: 480 };
    pub const DVD: Q = Q { id: 2, name: "DVD", source: S::Dvd, resolution: 480 };
    pub const WEBDL1080p: Q = Q { id: 3, name: "WEBDL-1080p", source: S::Web, resolution: 1080 };
    pub const HDTV720p: Q = Q { id: 4, name: "HDTV-720p", source: S::Television, resolution: 720 };
    pub const WEBDL720p: Q = Q { id: 5, name: "WEBDL-720p", source: S::Web, resolution: 720 };
    pub const Bluray720p: Q = Q { id: 6, name: "Bluray-720p", source: S::Bluray, resolution: 720 };
    pub const Bluray1080p: Q = Q { id: 7, name: "Bluray-1080p", source: S::Bluray, resolution: 1080 };
    pub const WEBDL480p: Q = Q { id: 8, name: "WEBDL-480p", source: S::Web, resolution: 480 };
    pub const HDTV1080p: Q = Q { id: 9, name: "HDTV-1080p", source: S::Television, resolution: 1080 };
    pub const RAWHD: Q = Q { id: 10, name: "Raw-HD", source: S::TelevisionRaw, resolution: 1080 };
    pub const WEBRip480p: Q = Q { id: 12, name: "WEBRip-480p", source: S::WebRip, resolution: 480 };
    pub const Bluray480p: Q = Q { id: 13, name: "Bluray-480p", source: S::Bluray, resolution: 480 };
    pub const WEBRip720p: Q = Q { id: 14, name: "WEBRip-720p", source: S::WebRip, resolution: 720 };
    pub const WEBRip1080p: Q = Q { id: 15, name: "WEBRip-1080p", source: S::WebRip, resolution: 1080 };
    pub const HDTV2160p: Q = Q { id: 16, name: "HDTV-2160p", source: S::Television, resolution: 2160 };
    pub const WEBRip2160p: Q = Q { id: 17, name: "WEBRip-2160p", source: S::WebRip, resolution: 2160 };
    pub const WEBDL2160p: Q = Q { id: 18, name: "WEBDL-2160p", source: S::Web, resolution: 2160 };
    pub const Bluray2160p: Q = Q { id: 19, name: "Bluray-2160p", source: S::Bluray, resolution: 2160 };
    pub const Bluray1080pRemux: Q = Q { id: 20, name: "Bluray-1080p Remux", source: S::BlurayRaw, resolution: 1080 };
    pub const Bluray2160pRemux: Q = Q { id: 21, name: "Bluray-2160p Remux", source: S::BlurayRaw, resolution: 2160 };
    pub const Bluray576p: Q = Q { id: 22, name: "Bluray-576p", source: S::Bluray, resolution: 576 };

    /// `Quality.All`, in Sonarr's order.
    pub const ALL: [Q; 22] = [
        Unknown,
        SDTV,
        DVD,
        WEBRip480p,
        WEBDL480p,
        Bluray480p,
        Bluray576p,
        HDTV720p,
        WEBRip720p,
        WEBDL720p,
        Bluray720p,
        Bluray1080p,
        HDTV1080p,
        WEBRip1080p,
        WEBDL1080p,
        RAWHD,
        HDTV2160p,
        WEBRip2160p,
        WEBDL2160p,
        Bluray2160p,
        Bluray1080pRemux,
        Bluray2160pRemux,
    ];
}

/// Radarr's qualities by name, as `Quality.<Name>` spells them upstream.
#[allow(non_upper_case_globals, missing_docs)]
pub mod movie {
    use super::{Modifier as M, MovieQuality as Q, MovieSource as S};
    const fn q(id: i64, name: &'static str, source: S, resolution: i64, modifier: M) -> Q {
        Q { id, name, source, resolution, modifier }
    }
    pub const Unknown: Q = q(0, "Unknown", S::Unknown, 0, M::None);
    pub const WORKPRINT: Q = q(24, "WORKPRINT", S::Workprint, 0, M::None);
    pub const CAM: Q = q(25, "CAM", S::Cam, 0, M::None);
    pub const TELESYNC: Q = q(26, "TELESYNC", S::Telesync, 0, M::None);
    pub const TELECINE: Q = q(27, "TELECINE", S::Telecine, 0, M::None);
    pub const DVDSCR: Q = q(28, "DVDSCR", S::Dvd, 480, M::Screener);
    pub const REGIONAL: Q = q(29, "REGIONAL", S::Dvd, 480, M::Regional);
    pub const SDTV: Q = q(1, "SDTV", S::Tv, 480, M::None);
    pub const DVD: Q = q(2, "DVD", S::Dvd, 0, M::None);
    pub const DVDR: Q = q(23, "DVD-R", S::Dvd, 480, M::Remux);
    pub const HDTV720p: Q = q(4, "HDTV-720p", S::Tv, 720, M::None);
    pub const HDTV1080p: Q = q(9, "HDTV-1080p", S::Tv, 1080, M::None);
    pub const HDTV2160p: Q = q(16, "HDTV-2160p", S::Tv, 2160, M::None);
    pub const WEBDL480p: Q = q(8, "WEBDL-480p", S::Webdl, 480, M::None);
    pub const WEBDL720p: Q = q(5, "WEBDL-720p", S::Webdl, 720, M::None);
    pub const WEBDL1080p: Q = q(3, "WEBDL-1080p", S::Webdl, 1080, M::None);
    pub const WEBDL2160p: Q = q(18, "WEBDL-2160p", S::Webdl, 2160, M::None);
    pub const Bluray480p: Q = q(20, "Bluray-480p", S::Bluray, 480, M::None);
    pub const Bluray576p: Q = q(21, "Bluray-576p", S::Bluray, 576, M::None);
    pub const Bluray720p: Q = q(6, "Bluray-720p", S::Bluray, 720, M::None);
    pub const Bluray1080p: Q = q(7, "Bluray-1080p", S::Bluray, 1080, M::None);
    pub const Bluray2160p: Q = q(19, "Bluray-2160p", S::Bluray, 2160, M::None);
    pub const Remux1080p: Q = q(30, "Remux-1080p", S::Bluray, 1080, M::Remux);
    pub const Remux2160p: Q = q(31, "Remux-2160p", S::Bluray, 2160, M::Remux);
    pub const BRDISK: Q = q(22, "BR-DISK", S::Bluray, 1080, M::BrDisk);
    pub const RAWHD: Q = q(10, "Raw-HD", S::Tv, 1080, M::RawHd);
    pub const WEBRip480p: Q = q(12, "WEBRip-480p", S::Webrip, 480, M::None);
    pub const WEBRip720p: Q = q(14, "WEBRip-720p", S::Webrip, 720, M::None);
    pub const WEBRip1080p: Q = q(15, "WEBRip-1080p", S::Webrip, 1080, M::None);
    pub const WEBRip2160p: Q = q(17, "WEBRip-2160p", S::Webrip, 2160, M::None);

    /// `Quality.All`, in Radarr's order.
    pub const ALL: [Q; 30] = [
        Unknown,
        WORKPRINT,
        CAM,
        TELESYNC,
        TELECINE,
        DVDSCR,
        REGIONAL,
        SDTV,
        DVD,
        DVDR,
        HDTV720p,
        HDTV1080p,
        HDTV2160p,
        WEBDL480p,
        WEBDL720p,
        WEBDL1080p,
        WEBDL2160p,
        WEBRip480p,
        WEBRip720p,
        WEBRip1080p,
        WEBRip2160p,
        Bluray480p,
        Bluray576p,
        Bluray720p,
        Bluray1080p,
        Bluray2160p,
        Remux1080p,
        Remux2160p,
        BRDISK,
        RAWHD,
    ];
}

impl TvQuality {
    /// The quality with Sonarr's id `id`.
    pub fn by_id(id: i64) -> Option<TvQuality> {
        tv::ALL.into_iter().find(|q| q.id == id)
    }

    /// `QualityFinder.FindBySourceAndResolution`: the quality with this source and resolution, else the nearest.
    pub fn find_by_source_and_resolution(source: TvSource, resolution: i64) -> TvQuality {
        if let Some(q) = tv::ALL.into_iter().find(|q| q.source == source && q.resolution == resolution) {
            return q;
        }
        // 576p releases from TV or the web aren't rolled up to Bluray-576p
        if resolution < 720 {
            match source {
                TvSource::Television => return tv::SDTV,
                TvSource::Web => return tv::WEBDL480p,
                TvSource::WebRip => return tv::WEBRip480p,
                _ => {}
            }
        }
        let mut same: Vec<TvQuality> = tv::ALL.into_iter().filter(|q| q.resolution == resolution).collect();
        same.sort_by_key(|q| q.source);
        same.into_iter().find(|q| q.source >= source).unwrap_or(tv::Unknown)
    }

    /// `MediaFileExtensions.GetQualityForExtension` (Sonarr's table): the quality a file extension suggests.
    pub fn for_extension(ext: &str) -> TvQuality {
        match ext.to_ascii_lowercase().as_str() {
            ".webm" => tv::Unknown,
            ".img" | ".iso" | ".vob" => tv::DVD,
            ".mkv" | ".ts" | ".wtv" => tv::HDTV720p,
            ".m2ts" => tv::Bluray720p,
            e if SD_EXTENSIONS.contains(&e) => tv::SDTV,
            _ => tv::Unknown,
        }
    }
}

impl MovieQuality {
    /// The quality with Radarr's id `id`.
    pub fn by_id(id: i64) -> Option<MovieQuality> {
        movie::ALL.into_iter().find(|q| q.id == id)
    }

    /// `QualityFinder.FindBySourceAndResolution` (Radarr's, with the modifier).
    pub fn find_by_source_and_resolution(source: MovieSource, resolution: i64, modifier: Modifier) -> MovieQuality {
        if let Some(q) = movie::ALL.into_iter().find(|q| q.source == source && q.resolution == resolution && q.modifier == modifier) {
            return q;
        }
        let unknown_res: Vec<MovieQuality> = movie::ALL
            .into_iter()
            .filter(|q| q.source == source && q.resolution == 0 && q.modifier == modifier && q.id != movie::Unknown.id)
            .collect();
        if unknown_res.len() == 1 {
            return unknown_res[0];
        }
        if let Some(q) = unknown_res.into_iter().find(|q| q.source >= source) {
            return q;
        }
        let mut same: Vec<MovieQuality> = movie::ALL.into_iter().filter(|q| q.modifier == modifier && q.resolution == resolution).collect();
        same.sort_by_key(|q| q.source);
        same.into_iter().find(|q| q.source >= source).unwrap_or(movie::Unknown)
    }

    /// `MediaFileExtensions.GetQualityForExtension` (Radarr's table).
    pub fn for_extension(ext: &str) -> MovieQuality {
        match ext.to_ascii_lowercase().as_str() {
            ".webm" => movie::Unknown,
            ".img" | ".iso" | ".vob" => movie::DVD,
            ".mkv" | ".mk3d" => movie::WEBDL720p,
            ".ts" | ".wtv" => movie::SDTV,
            ".m2ts" => movie::Bluray720p,
            e if SD_EXTENSIONS.contains(&e) => movie::SDTV,
            _ => movie::Unknown,
        }
    }
}

/// The extensions both apps read as SDTV.
const SD_EXTENSIONS: &[&str] = &[
    ".m4v", ".3gp", ".nsv", ".ty", ".strm", ".rm", ".rmvb", ".m3u", ".ifo", ".mov", ".qt", ".divx", ".xvid", ".bivx", ".nrg", ".pva",
    ".wmv", ".asf", ".asx", ".ogm", ".ogv", ".m2v", ".avi", ".bin", ".dat", ".dvr-ms", ".mpg", ".mpeg", ".mp4", ".avc", ".vp3", ".svq3",
    ".nuv", ".viv", ".dv", ".fli", ".flv", ".wpl",
];

/// Every extension Sonarr and Radarr take as a video file (`MediaFileExtensions.Extensions`).
pub fn is_video_extension(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    SD_EXTENSIONS.contains(&e.as_str()) || [".webm", ".img", ".iso", ".vob", ".mkv", ".mk3d", ".ts", ".wtv", ".m2ts"].contains(&e.as_str())
}

/// How a part of the quality was found (`QualityDetectionSource`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DetectionSource {
    /// not found
    #[default]
    Unknown,
    /// from the release name
    Name,
    /// from the file extension
    Extension,
    /// from the file's own media information
    MediaInfo,
}

/// A release's revision: proper, repack and REAL count (`Revision`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Revision {
    /// 1, 2 for a proper or repack, more for a numbered repack or anime `v3`
    pub version: i64,
    /// how many times the name says REAL
    pub real: i64,
    /// a repack or rerip
    pub is_repack: bool,
}

impl Default for Revision {
    fn default() -> Self {
        Revision { version: 1, real: 0, is_repack: false }
    }
}

impl PartialOrd for Revision {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Revision {
    /// `Revision.CompareTo`: REAL first, then the version.
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.real.cmp(&other.real).then(self.version.cmp(&other.version))
    }
}

/// A quality and how it was found (`QualityModel`); `Q` is [`TvQuality`] or [`MovieQuality`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualityModel<Q> {
    /// the quality
    pub quality: Q,
    /// proper, repack, REAL
    pub revision: Revision,
    /// how the source was found
    pub source_detection: DetectionSource,
    /// how the resolution was found
    pub resolution_detection: DetectionSource,
    /// how the modifier was found (Radarr)
    pub modifier_detection: DetectionSource,
    /// how the revision was found
    pub revision_detection: DetectionSource,
}

impl<Q> QualityModel<Q> {
    /// A model for `quality` with nothing found yet.
    pub fn new(quality: Q) -> Self {
        QualityModel {
            quality,
            revision: Revision::default(),
            source_detection: DetectionSource::Unknown,
            resolution_detection: DetectionSource::Unknown,
            modifier_detection: DetectionSource::Unknown,
            revision_detection: DetectionSource::Unknown,
        }
    }
}

/// `Path.GetExtension` as .NET answers on Linux: from the last `.` after the last `/`, "" when there is none or it
/// ends the name.
pub fn path_extension(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    match file.rfind('.') {
        Some(i) if i + 1 < file.len() => &file[i..],
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finder_falls_back_like_upstream() {
        assert_eq!(TvQuality::find_by_source_and_resolution(TvSource::Television, 576), tv::SDTV);
        assert_eq!(TvQuality::find_by_source_and_resolution(TvSource::Bluray, 1080), tv::Bluray1080p);
        assert_eq!(TvQuality::find_by_source_and_resolution(TvSource::Dvd, 1080), tv::Bluray1080p);
        assert_eq!(MovieQuality::find_by_source_and_resolution(MovieSource::Bluray, 1080, Modifier::Remux), movie::Remux1080p);
        assert_eq!(MovieQuality::find_by_source_and_resolution(MovieSource::Dvd, 480, Modifier::None), movie::DVD);
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in tv::ALL.iter().enumerate() {
            assert!(tv::ALL[i + 1..].iter().all(|b| b.id != a.id));
        }
        for (i, a) in movie::ALL.iter().enumerate() {
            assert!(movie::ALL[i + 1..].iter().all(|b| b.id != a.id));
        }
    }
}
