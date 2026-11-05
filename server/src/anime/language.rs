//! What an anime request wants to hear and read (dub, dual audio or subtitles), and checking a downloaded file's
//! real audio and subtitle tracks with ffprobe against it.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Which audio an anime request wants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Audio {
    /// only the dub (for example English audio), the original audio isn't needed
    Dub,
    /// the original audio and the dub
    Dual,
    /// the original (Japanese) audio, with subtitles
    #[default]
    Sub,
}

impl Audio {
    /// Reads `dub`, `dual` or `sub`.
    pub fn parse(s: &str) -> Option<Audio> {
        match s.trim().to_lowercase().as_str() {
            "dub" => Some(Audio::Dub),
            "dual" => Some(Audio::Dual),
            "sub" => Some(Audio::Sub),
            _ => None,
        }
    }

    /// The word for it: `dub`, `dual` or `sub`.
    pub fn word(self) -> &'static str {
        match self {
            Audio::Dub => "dub",
            Audio::Dual => "dual",
            Audio::Sub => "sub",
        }
    }

    /// In words.
    pub fn label(self) -> &'static str {
        match self {
            Audio::Dub => "Dub only",
            Audio::Dual => "Dual audio",
            Audio::Sub => "Sub only",
        }
    }
}

/// A language choice: the audio, the dub's language and the subtitles' language (ISO 639-2, `eng`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    /// dub, dual or sub
    pub audio: Audio,
    /// the dub's language
    pub dub_language: String,
    /// the subtitles' language; "" for none needed
    pub subtitles: String,
}

impl Default for Choice {
    fn default() -> Choice {
        Choice { audio: Audio::Sub, dub_language: "eng".into(), subtitles: "eng".into() }
    }
}

impl Choice {
    /// In words, for request pages: `Dual audio, English subtitles`.
    pub fn words(&self) -> String {
        let lang = |c: &str| language_name(c).to_string();
        match self.audio {
            Audio::Dub => format!("Dub only ({})", lang(&self.dub_language)),
            Audio::Dual => {
                if self.subtitles.is_empty() {
                    format!("Dual audio (Japanese and {})", lang(&self.dub_language))
                } else {
                    format!("Dual audio (Japanese and {}), {} subtitles", lang(&self.dub_language), lang(&self.subtitles))
                }
            }
            Audio::Sub => {
                format!("Sub only, {} subtitles", lang(&if self.subtitles.is_empty() { "eng".to_string() } else { self.subtitles.clone() }))
            }
        }
    }
}

/// The audio and subtitle languages a file holds (ISO 639-2; `und` when a track doesn't say).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Tracks {
    /// audio languages, in track order
    pub audio: Vec<String>,
    /// subtitle languages, in track order (forced-only tracks left out)
    pub subtitles: Vec<String>,
}

/// The common two-letter and other spellings as ISO 639-2 (`en`, `english` → `eng`).
pub fn normalize(code: &str) -> String {
    let c = code.trim().to_lowercase();
    match c.as_str() {
        "en" | "english" => "eng",
        "ja" | "jp" | "japanese" => "jpn",
        "de" | "ger" | "german" => "deu",
        "fr" | "fre" | "french" => "fra",
        "es" | "spanish" => "spa",
        "it" | "italian" => "ita",
        "pt" | "portuguese" => "por",
        "zh" | "chi" | "chinese" => "zho",
        "ko" | "korean" => "kor",
        "" => "und",
        other => return other.to_string(),
    }
    .to_string()
}

/// A language as two letters (ISO 639-1, `en`), the way the request pages keep it; `en` when unknown.
pub fn two_letter(code: &str) -> String {
    match normalize(code).as_str() {
        "eng" => "en",
        "jpn" => "ja",
        "deu" => "de",
        "fra" => "fr",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "zho" => "zh",
        "kor" => "ko",
        c if c.len() == 2 => return c.to_string(),
        _ => "en",
    }
    .to_string()
}

/// A language's name in words.
pub fn language_name(code: &str) -> &'static str {
    match normalize(code).as_str() {
        "eng" => "English",
        "jpn" => "Japanese",
        "deu" => "German",
        "fra" => "French",
        "spa" => "Spanish",
        "ita" => "Italian",
        "por" => "Portuguese",
        "zho" => "Chinese",
        "kor" => "Korean",
        _ => "another language",
    }
}

/// Reads ffprobe's JSON (`-show_entries stream=codec_type:stream_tags=language:stream_disposition=forced`).
pub fn read_ffprobe(v: &Value) -> Tracks {
    let mut t = Tracks::default();
    for s in v["streams"].as_array().cloned().unwrap_or_default() {
        let lang = normalize(s["tags"]["language"].as_str().unwrap_or(""));
        match s["codec_type"].as_str() {
            Some("audio") => t.audio.push(lang),
            Some("subtitle") if s["disposition"]["forced"].as_i64() != Some(1) => t.subtitles.push(lang),
            _ => {}
        }
    }
    t
}

/// Asks ffprobe for a file's tracks (run with an argument list, never a shell).
pub fn probe(file: &Path) -> Result<Tracks, String> {
    let out = std::process::Command::new(std::env::var("MARQUEE_FFPROBE").unwrap_or_else(|_| "ffprobe".into()))
        .args(["-v", "error", "-show_entries", "stream=codec_type:stream_tags=language:stream_disposition=forced", "-of", "json"])
        .arg(file)
        .output()
        .map_err(|e| format!("ffprobe can't be run to check the tracks: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffprobe couldn't read {}: {}", file.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    let v: Value = serde_json::from_slice(&out.stdout).map_err(|_| "ffprobe's answer can't be read".to_string())?;
    Ok(read_ffprobe(&v))
}

/// Why a file doesn't give what the choice asks for, in words; `None` when it does. Tracks that don't say their
/// language count as the original audio when they are the only audio.
pub fn check(c: &Choice, t: &Tracks) -> Option<String> {
    let dub = normalize(&c.dub_language);
    let has_audio = |l: &str| t.audio.iter().any(|a| a == l);
    let japanese = has_audio("jpn") || (t.audio.len() == 1 && t.audio[0] == "und");
    let subs = normalize(if c.subtitles.is_empty() { "eng" } else { &c.subtitles });
    let has_subs = t.subtitles.contains(&subs);
    let heard = if t.audio.is_empty() {
        "no audio".to_string()
    } else {
        t.audio.iter().map(|a| language_name(a)).collect::<Vec<_>>().join(" and ")
    };
    match c.audio {
        Audio::Dub if !has_audio(&dub) => Some(format!("It has {heard} audio, but {} audio was asked for", language_name(&dub))),
        Audio::Dual if !japanese || !has_audio(&dub) => {
            Some(format!("It has {heard} audio, but Japanese and {} audio were asked for", language_name(&dub)))
        }
        Audio::Dual if !c.subtitles.is_empty() && !has_subs => Some(format!("It has no {} subtitles", language_name(&subs))),
        Audio::Sub if !japanese => Some(format!("It has {heard} audio, but the Japanese audio was asked for")),
        Audio::Sub if !has_subs => Some(format!("It has no {} subtitles", language_name(&subs))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn probe_json(audio: &[&str], subs: &[(&str, bool)]) -> Value {
        let mut s = vec![json!({"codec_type": "video", "tags": {}})];
        s.extend(audio.iter().map(|a| json!({"codec_type": "audio", "tags": {"language": a}, "disposition": {"forced": 0}})));
        s.extend(
            subs.iter().map(|(l, f)| json!({"codec_type": "subtitle", "tags": {"language": l}, "disposition": {"forced": *f as i64}})),
        );
        json!({ "streams": s })
    }

    #[test]
    fn tracks_are_read_from_ffprobe() {
        let t = read_ffprobe(&probe_json(&["jpn", "en"], &[("eng", false), ("eng", true)]));
        assert_eq!(t, Tracks { audio: vec!["jpn".into(), "eng".into()], subtitles: vec!["eng".into()] });
    }

    #[test]
    fn files_are_checked_against_the_choice() {
        let dub = Choice { audio: Audio::Dub, ..Choice::default() };
        let dual = Choice { audio: Audio::Dual, subtitles: String::new(), ..Choice::default() };
        let sub = Choice::default();
        let jp_only = read_ffprobe(&probe_json(&["jpn"], &[("eng", false)]));
        let both = read_ffprobe(&probe_json(&["jpn", "eng"], &[]));
        assert_eq!(check(&dub, &jp_only).as_deref(), Some("It has Japanese audio, but English audio was asked for"));
        assert_eq!(check(&dub, &both), None);
        assert_eq!(check(&dual, &both), None);
        assert!(check(&dual, &jp_only).unwrap().contains("Japanese and English audio were asked for"));
        assert_eq!(check(&sub, &jp_only), None);
        assert_eq!(check(&sub, &both).as_deref(), Some("It has no English subtitles"));
        let unknown = read_ffprobe(&probe_json(&[""], &[("eng", false)]));
        assert_eq!(check(&sub, &unknown), None, "a lone untagged track counts as the original");
        assert_eq!(Choice::default().words(), "Sub only, English subtitles");
        assert_eq!(dual.words(), "Dual audio (Japanese and English)");
    }

    #[test]
    fn ffprobe_reads_a_real_file() {
        let f = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/dual_sub.mkv");
        match probe(&f) {
            Ok(t) => assert_eq!(t, Tracks { audio: vec!["jpn".into(), "eng".into()], subtitles: vec!["eng".into()] }),
            // the check is the same without ffprobe on the machine; the image ships it
            Err(e) => assert!(e.contains("can't be run"), "{e}"),
        }
    }
}
