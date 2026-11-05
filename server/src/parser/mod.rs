//! Ports of Sonarr's and Radarr's `NzbDrone.Core.Parser`: reading release and file names. The upstream regular
//! expressions are kept as they are (see [`net_regex`]) and the upstream NUnit cases run as Rust tests
//! (`server/tests/parser.rs`, cases in `server/tests/upstream/`).
//!
//! | Upstream | Here |
//! | --- | --- |
//! | `Qualities/*`, `MediaFiles/MediaFileExtensions` | [`quality`] |
//! | Sonarr `Parser/QualityParser` | [`quality_tv`] |
//! | Radarr `Parser/QualityParser` | [`quality_movie`] |
//! | `Languages/Language`, `Parser/IsoLanguage(s)`, subtitle half of `Parser/LanguageParser` | [`language`] |
//! | Sonarr `Parser/LanguageParser.ParseLanguages` | [`language_tv`] |
//! | Radarr `Parser/LanguageParser.ParseLanguages` | [`language_movie`] |

pub mod language;
pub mod language_movie;
pub mod language_tv;
pub mod net_regex;
pub mod quality;
pub mod quality_movie;
pub mod quality_tv;
