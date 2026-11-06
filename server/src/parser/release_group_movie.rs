//! Radarr's `ReleaseGroupParser`, `ParserCommon` and `MediaFiles/FileExtensions.RemoveFileExtension`: the group
//! that made a movie release. Port of the files at the Radarr commit in `UPSTREAM.lock`; the expressions are the
//! upstream ones, unchanged.

use super::net_regex::{NetRegex, I};
use super::quality::is_video_extension;
use super::text::dotnet_int;
use std::sync::LazyLock;

/// `ParserCommon.WebsitePrefixRegex`: a site name in front (`[www.site.com] - `).
pub(super) static WEBSITE_PREFIX: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"^(?:(?:\[|\()\s*)?(?:www\.)?[-a-z0-9-]{1,256}\.(?<!Naruto-Kun\.)(?:[a-z]{2,6}\.[a-z]{2,6}|xn--[a-z0-9-]{4,}|[a-z]{2,})\b(?:\s*(?:\]|\))|[ -]{2,})[ -]*",
        I,
    )
});

/// `ParserCommon.WebsitePostfixRegex`: a site name at the end (`[www.site.com]`).
pub(super) static WEBSITE_POSTFIX: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"(?:\[\s*)?(?:www\.)?[-a-z0-9-]{1,256}\.(?:xn--[a-z0-9-]{4,}|[a-z]{2,6})\b(?:\s*\])$", I));

/// `ParserCommon.CleanTorrentSuffixRegex`: a tracker tag at the end.
pub(super) static CLEAN_TORRENT_SUFFIX: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\[(?:ettv|rartv|rarbg|cttv|publichd)\]$", I));

/// `FileExtensions.FileExtensionRegex`
static FILE_EXTENSION: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"\.[a-z0-9]{2,4}$", I));

/// The name without a video or Usenet file extension (`FileExtensions.RemoveFileExtension`); other extensions
/// stay.
pub fn remove_file_extension(title: &str) -> String {
    FILE_EXTENSION.replace_all_with(title, |c| {
        let ext = c.whole().text.to_lowercase();
        if is_video_extension(&ext) || ext == ".par2" || ext == ".nzb" {
            String::new()
        } else {
            c.whole().text.to_string()
        }
    })
}

/// `ReleaseGroupRegex`
static RELEASE_GROUP: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"-(?<releasegroup>[a-z0-9]+(?<part2>-[a-z0-9]+)?(?!.+?(?:480p|576p|720p|1080p|2160p)))(?<!(?:WEB-(DL|Rip)|Blu-Ray|480p|576p|720p|1080p|2160p|DTS-HD|DTS-X|DTS-MA|DTS-ES|-ES|-EN|-CAT|-ENG|-JAP|-GER|-FRA|-FRE|-ITA|-HDRip|\d{1,2}-bit|[ ._]\d{4}-\d{2}|-\d{2}|tmdb(id)?-(?<tmdbid>\d+)|(?<imdbid>tt\d{7,8}))(?:\k<part2>)?)(?:\b|[-._ ]|$)|[-._ ]\[(?<releasegroup>[a-z0-9]+)\]$",
        I,
    )
});

/// `InvalidReleaseGroupRegex`
static INVALID_RELEASE_GROUP: LazyLock<NetRegex> = LazyLock::new(|| NetRegex::new(r"^([se]\d+|[0-9a-f]{8})$", I));

/// `AnimeReleaseGroupRegex`
static ANIME_RELEASE_GROUP: LazyLock<NetRegex> =
    LazyLock::new(|| NetRegex::new(r"^(?:\[(?<subgroup>(?!\s).+?(?<!\s))\](?:_|-|\s|\.)?)", I));

/// `ExceptionReleaseGroupRegexExact`: groups that don't use `-Group`.
static EXCEPTION_RELEASE_GROUP_EXACT: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"\b(?<releasegroup>KRaLiMaRKo|E\.N\.D|D\-Z0N3|Koten_Gars|BluDragon|ZØNEHD|HQMUX|VARYG|YIFY|YTS(.(MX|LT|AG))?|TMd|Eml HDTeam|LMain|DarQ|BEN THE MEN|TAoE|QxR|126811)\b",
        I,
    )
});

/// `ExceptionReleaseGroupRegex`: groups whose releases end with `Group)` or `Group]`.
static EXCEPTION_RELEASE_GROUP: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(?<=[._ \[])(?<releasegroup>(Silence|afm72|Panda|Ghost|MONOLITH|Tigole|Joy|ImE|UTR|t3nzin|Anime Time|Project Angel|Hakata Ramen|HONE|GiLG|Vyndros|SEV|Garshasp|Kappa|Natty|RCVR|SAMPA|YOGI|r00t|EDGE2020|RZeroX|FreetheFish|Anna|Bandi|Qman|theincognito|HDO|DusIctv|DHD|CtrlHD|-ZR-|ADC|XZVN|RH|Kametsu|Celdra)(?=\]|\)))",
        I,
    )
});

/// `CleanReleaseGroupRegex`: posting-tool suffixes that aren't the group.
static CLEAN_RELEASE_GROUP: LazyLock<NetRegex> = LazyLock::new(|| {
    NetRegex::new(
        r"(-(RP|1|NZBGeek|Obfuscated|Obfuscation|Scrambled|sample|Pre|postbot|xpost|Rakuv[a-z0-9]*|WhiteRev|BUYMORE|AsRequested|AlternativeToRequested|GEROV|Z0iDS3N|Chamele0n|4P|4Planet|AlteZachen|RePACKPOST))+$",
        I,
    )
});

/// The release group of a movie release name (`ParseReleaseGroup`); `None` when there is none.
pub fn parse_release_group(title: &str) -> Option<String> {
    let title = remove_file_extension(title.trim());
    // `ParserCommon.PreSubstitutionRegex` is empty in Radarr
    let title = WEBSITE_PREFIX.replace_all(&title, "");
    let title = CLEAN_TORRENT_SUFFIX.replace_all(&title, "");
    if let Some(m) = ANIME_RELEASE_GROUP.captures(&title) {
        return Some(m.str("subgroup").to_string());
    }
    let title = CLEAN_RELEASE_GROUP.replace_all(&title, "");
    if let Some(m) = EXCEPTION_RELEASE_GROUP_EXACT.captures_iter(&title).last() {
        return Some(m.str("releasegroup").to_string());
    }
    if let Some(m) = EXCEPTION_RELEASE_GROUP.captures_iter(&title).last() {
        return Some(m.str("releasegroup").to_string());
    }
    let m = RELEASE_GROUP.captures_iter(&title).pop()?;
    let group = m.str("releasegroup");
    if dotnet_int(group).is_some() || INVALID_RELEASE_GROUP.is_match(group) {
        return None;
    }
    Some(group.to_string())
}
