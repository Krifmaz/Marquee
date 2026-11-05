//! Small .NET string behaviours the ported parsers rely on: `int.TryParse`, UTF-16 lengths and positions, and
//! `RemoveAccent` from `NzbDrone.Common.Extensions.StringExtensions`.

use unicode_normalization::UnicodeNormalization;

/// `int.TryParse` with .NET's default style: spaces around, an optional sign, digits, within `i32`.
pub fn dotnet_int(s: &str) -> Option<i32> {
    dotnet_long(s).and_then(|n| i32::try_from(n).ok())
}

/// `long.TryParse` with .NET's default style.
pub fn dotnet_long(s: &str) -> Option<i64> {
    let t = s.trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.strip_prefix('+').unwrap_or(t).parse().ok()
}

/// The length .NET gives a string: UTF-16 code units.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// The byte position in `s` of UTF-16 position `pos`, or `None` past the end (where .NET throws).
pub fn byte_at_utf16(s: &str, pos: usize) -> Option<usize> {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        if units >= pos {
            return Some(i);
        }
        units += c.len_utf16();
    }
    (units >= pos).then_some(s.len())
}

/// The text without accents (`RemoveAccent`): decomposed, nonspacing marks dropped, composed again.
pub fn remove_accent(s: &str) -> String {
    use std::sync::LazyLock;
    static MARK: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\p{Mn}").expect("valid"));
    let decomposed: String = s.nfd().collect();
    MARK.replace_all(&decomposed, "").nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_parse_like_dotnet() {
        assert_eq!(dotnet_int(" 12 "), Some(12));
        assert_eq!(dotnet_int("+3"), Some(3));
        assert_eq!(dotnet_int("-3"), Some(-3));
        assert_eq!(dotnet_int("1a"), None);
        assert_eq!(dotnet_int("99999999999"), None);
        assert_eq!(dotnet_int(""), None);
    }

    #[test]
    fn accents_and_utf16_positions() {
        assert_eq!(remove_accent("Carnivàle"), "Carnivale");
        assert_eq!(utf16_len("a😀"), 3);
        assert_eq!(byte_at_utf16("é.x", 1), Some(2));
        assert_eq!(byte_at_utf16("ab", 3), None);
    }
}
