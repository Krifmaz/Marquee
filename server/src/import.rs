//! Putting finished downloads into the library: picking the video files, naming them like Radarr and Sonarr do, and
//! hardlinking them into the movie or series folder (a copy only when the download is on another disk), so the
//! download keeps seeding without taking the space twice. A file it replaces goes to the root folder's
//! `.marquee-recycle` folder, never deleted.

use std::path::{Path, PathBuf};

/// File endings that are videos.
pub const VIDEO: [&str; 9] = ["mkv", "mp4", "avi", "m4v", "ts", "mov", "wmv", "webm", "mpg"];

/// File endings of subtitles brought along with their video.
pub const SUBTITLES: [&str; 4] = ["srt", "ass", "ssa", "sub"];

/// How a file got into the library.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placed {
    /// a hardlink: the same file, no extra space
    Hardlink,
    /// a copy, because the download is on another disk
    Copy,
}

/// The video files among `paths` (folders are walked), samples left out unless there is nothing else; the
/// biggest first.
pub fn videos(paths: &[String]) -> Vec<PathBuf> {
    let mut all = Vec::new();
    for p in paths {
        walk(Path::new(p), &mut all, 0);
    }
    let is_video =
        |p: &PathBuf| p.extension().and_then(|e| e.to_str()).map(|e| VIDEO.contains(&e.to_lowercase().as_str())).unwrap_or(false);
    let mut v: Vec<PathBuf> = all.into_iter().filter(is_video).collect();
    let sample = |p: &PathBuf| p.to_string_lossy().to_lowercase().contains("sample");
    if v.iter().any(|p| !sample(p)) {
        v.retain(|p| !sample(p));
    }
    v.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)));
    v
}

/// Every file under `p` (a file is itself), at most 4 folders deep.
fn walk(p: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if p.is_file() {
        out.push(p.to_path_buf());
    } else if p.is_dir() && depth < 4 {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                walk(&e.path(), out, depth + 1);
            }
        }
    }
}

/// A name safe on every disk: `:` becomes ` -`, and `/ \ ? * " < > |` are dropped.
pub fn safe(name: &str) -> String {
    let n = name.replace(": ", " - ").replace(':', "-");
    let n: String = n.chars().filter(|c| !matches!(c, '/' | '\\' | '?' | '*' | '"' | '<' | '>' | '|') && !c.is_control()).collect();
    n.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('.').to_string()
}

/// A movie file's name: `Title (Year) Quality` (Radarr's standard format) and the video's ending.
pub fn movie_file_name(title: &str, year: i64, quality: &str, source: &Path) -> String {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("mkv").to_lowercase();
    if year > 0 {
        safe(&format!("{title} ({year}) {quality}")) + "." + &ext
    } else {
        safe(&format!("{title} {quality}")) + "." + &ext
    }
}

/// An episode file's name: `Series - S01E02 - Title Quality` (Sonarr's standard format; several episodes as
/// `S01E01-E02` with the titles joined by ` + `).
pub fn episode_file_name(series: &str, season: i64, episodes: &[(i64, String)], quality: &str, source: &Path) -> String {
    let ext = source.extension().and_then(|e| e.to_str()).unwrap_or("mkv").to_lowercase();
    let mut nums = format!("S{season:02}E{:02}", episodes.first().map(|e| e.0).unwrap_or(0));
    if episodes.len() > 1 {
        nums += &format!("-E{:02}", episodes.last().map(|e| e.0).unwrap_or(0));
    }
    let titles: Vec<&str> = episodes.iter().map(|e| e.1.as_str()).filter(|t| !t.is_empty()).collect();
    let title = if titles.is_empty() { String::new() } else { format!(" - {}", titles.join(" + ")) };
    safe(&format!("{series} - {nums}{title} {quality}")) + "." + &ext
}

/// The season folder's name: `Season 1`, or `Specials` for season 0.
pub fn season_folder(season: i64) -> String {
    if season == 0 {
        "Specials".into()
    } else {
        format!("Season {season}")
    }
}

/// Puts `from` at `to`: a hardlink, or a copy when they are on different disks. The folder is made; a file
/// already at `to` that is the same file is kept as it is.
pub fn place(from: &Path, to: &Path) -> std::io::Result<Placed> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if to.exists() {
        if same_file(from, to) {
            return Ok(Placed::Hardlink);
        }
        return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} is there already", to.display())));
    }
    match std::fs::hard_link(from, to) {
        Ok(()) => Ok(Placed::Hardlink),
        Err(e) => {
            tracing::info!("{} can't be hardlinked to {} ({e}); copying it", from.display(), to.display());
            std::fs::copy(from, to)?;
            Ok(Placed::Copy)
        }
    }
}

/// Whether two paths are the same file on disk.
#[cfg(unix)]
pub fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

/// Whether two paths are the same file on disk (by size and time where inodes aren't known).
#[cfg(not(unix))]
pub fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() == y.modified().ok(),
        _ => false,
    }
}

/// Moves a file that is being replaced into `<root>/.marquee-recycle/`, keeping its name; where it went.
pub fn recycle(file: &Path, root: &Path) -> std::io::Result<PathBuf> {
    let bin = root.join(".marquee-recycle");
    std::fs::create_dir_all(&bin)?;
    let name = file.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    let mut to = bin.join(&name);
    let mut n = 1;
    while to.exists() {
        to = bin.join(format!("{n} {}", name.to_string_lossy()));
        n += 1;
    }
    std::fs::rename(file, &to)?;
    Ok(to)
}

/// The subtitles next to `video` in its download (same name start), to bring along.
pub fn subtitles_of(video: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str())) else {
        return vec![];
    };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
            SUBTITLES.contains(&ext.as_str()) && p.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with(stem)).unwrap_or(false)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_like_radarr_and_sonarr() {
        let src = Path::new("/d/x.MKV");
        assert_eq!(movie_file_name("Mission: Impossible", 1996, "Bluray-1080p", src), "Mission - Impossible (1996) Bluray-1080p.mkv");
        assert_eq!(
            episode_file_name("The Bear", 1, &[(2, "Hands".into())], "WEBDL-1080p", src),
            "The Bear - S01E02 - Hands WEBDL-1080p.mkv"
        );
        assert_eq!(
            episode_file_name("Show", 2, &[(1, "A".into()), (2, "B".into())], "HDTV-720p", src),
            "Show - S02E01-E02 - A + B HDTV-720p.mkv"
        );
        assert_eq!(season_folder(0), "Specials");
        assert_eq!(safe("What? / Why*"), "What Why");
    }

    #[test]
    fn files_are_hardlinked_and_replaced_files_recycled() {
        let dir = std::env::temp_dir().join(format!("marquee-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dl = dir.join("downloads/Movie.2020.1080p.WEB-DL-GRP");
        std::fs::create_dir_all(dl.join("Sample")).unwrap();
        std::fs::write(dl.join("movie.2020.1080p.mkv"), vec![0u8; 2048]).unwrap();
        std::fs::write(dl.join("movie.2020.1080p.en.srt"), "1").unwrap();
        std::fs::write(dl.join("Sample/sample.mkv"), vec![0u8; 10]).unwrap();
        std::fs::write(dl.join("info.nfo"), "x").unwrap();
        let v = videos(&[dl.to_string_lossy().to_string()]);
        assert_eq!(v, vec![dl.join("movie.2020.1080p.mkv")]);
        assert_eq!(subtitles_of(&v[0]), vec![dl.join("movie.2020.1080p.en.srt")]);
        let to = dir.join("media/movies/Movie (2020)/Movie (2020) WEBDL-1080p.mkv");
        assert_eq!(place(&v[0], &to).unwrap(), Placed::Hardlink);
        assert!(same_file(&v[0], &to));
        assert_eq!(place(&v[0], &to).unwrap(), Placed::Hardlink, "placing it again is fine");
        let gone = recycle(&to, &dir.join("media/movies")).unwrap();
        assert!(!to.exists() && gone.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
