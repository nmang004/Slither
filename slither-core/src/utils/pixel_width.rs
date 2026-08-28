/// Estimates the pixel width of a title tag as rendered in Google SERP results.
///
/// Uses Arial ~20px font metrics for title display.
pub fn estimate_title_pixel_width(text: &str) -> u32 {
    estimate_pixel_width(text, CharWidthProfile::Title)
}

/// Estimates the pixel width of a meta description as rendered in Google SERP results.
///
/// Uses Arial ~14px font metrics for description display.
pub fn estimate_description_pixel_width(text: &str) -> u32 {
    estimate_pixel_width(text, CharWidthProfile::Description)
}

#[derive(Clone, Copy)]
enum CharWidthProfile {
    /// Arial ~20px (title tag in SERP)
    Title,
    /// Arial ~14px (meta description in SERP)
    Description,
}

impl CharWidthProfile {
    fn wide(self) -> u32 {
        match self {
            Self::Title => 17,
            Self::Description => 12,
        }
    }

    fn uppercase(self) -> u32 {
        match self {
            Self::Title => 13,
            Self::Description => 9,
        }
    }

    fn lowercase(self) -> u32 {
        match self {
            Self::Title => 11,
            Self::Description => 8,
        }
    }

    fn narrow(self) -> u32 {
        match self {
            Self::Title => 6,
            Self::Description => 4,
        }
    }

    fn digit(self) -> u32 {
        match self {
            Self::Title => 11,
            Self::Description => 8,
        }
    }

    fn space(self) -> u32 {
        match self {
            Self::Title => 5,
            Self::Description => 4,
        }
    }

    fn default_width(self) -> u32 {
        match self {
            Self::Title => 11,
            Self::Description => 8,
        }
    }
}

fn estimate_pixel_width(text: &str, profile: CharWidthProfile) -> u32 {
    text.chars().map(|c| char_width(c, profile)).sum()
}

/// True for characters rendered at roughly double a Latin character's width:
/// CJK ideographs, kana, Hangul, and full-width forms.
///
/// Without this they fall through to the Latin default, so a Chinese title is
/// measured at about half its real rendered width.
pub fn is_full_width(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x115F   // Hangul Jamo
        | 0x2E80..=0x303E // CJK radicals, Kangxi, CJK symbols and punctuation
        | 0x3041..=0x33FF // Kana, Bopomofo, Hangul Compatibility Jamo, CJK compat
        | 0x3400..=0x4DBF // CJK Unified Ideographs Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xA000..=0xA4CF // Yi
        | 0xAC00..=0xD7A3 // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility ideographs
        | 0xFE30..=0xFE6F // CJK compatibility forms
        | 0xFF00..=0xFF60 // Full-width forms
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x2FA1F // CJK extensions B-F
    )
}

/// True when text is mostly full-width, so Latin-centric character-count rules
/// (like "titles under 30 characters are too short") do not apply.
pub fn is_full_width_text(text: &str) -> bool {
    let mut wide = 0usize;
    let mut total = 0usize;
    for c in text.chars().filter(|c| !c.is_whitespace()) {
        total += 1;
        if is_full_width(c) {
            wide += 1;
        }
    }
    total > 0 && wide * 2 >= total
}

/// True for characters that occupy no horizontal space when rendered:
/// zero-width joiners and spaces, the BOM, directional marks, variation
/// selectors, and combining marks (which render on top of the preceding glyph).
///
/// Charging these the default 11px each inflated a title carrying a stray BOM —
/// a common CMS template-concatenation artefact — by over 20%, pushing it across
/// the `over_561_pixels` threshold for no visible reason. A ZWJ emoji family
/// sequence is seven code points but one glyph.
pub fn is_zero_width(c: char) -> bool {
    matches!(c as u32,
        0x00AD             // soft hyphen
        | 0x200B..=0x200F  // ZWSP, ZWNJ, ZWJ, LRM, RLM
        | 0x202A..=0x202E  // bidi embedding/override
        | 0x2060..=0x2064  // word joiner, invisible operators
        | 0xFEFF           // BOM / zero-width no-break space
        | 0xFE00..=0xFE0F  // variation selectors
        | 0x0300..=0x036F  // combining diacritical marks
        | 0x1AB0..=0x1AFF  // combining diacritical marks extended
        | 0x20D0..=0x20FF  // combining marks for symbols
    )
}

fn char_width(c: char, profile: CharWidthProfile) -> u32 {
    if is_zero_width(c) {
        return 0;
    }
    if is_full_width(c) {
        // A full-width glyph occupies about two Latin characters.
        return profile.lowercase() * 2;
    }
    match c {
        // Wide characters
        'M' | 'W' | 'm' | 'w' => profile.wide(),
        // Narrow characters
        'i' | 'l' | 't' | 'f' | 'j' | '1' | '!' | '.' | ',' | ':' | ';' | '|' | '\'' | '"' => {
            profile.narrow()
        }
        // Space
        ' ' => profile.space(),
        // Digits (except 1, handled above)
        '0' | '2'..='9' => profile.digit(),
        // Uppercase letters (except wide ones handled above)
        'A'..='L' | 'N'..='V' | 'X'..='Z' => profile.uppercase(),
        // Lowercase letters (except wide and narrow ones handled above)
        'a'..='h' | 'k' | 'n' | 'o' | 'p'..='s' | 'u' | 'v' | 'x' | 'y' | 'z' => {
            profile.lowercase()
        }
        // Default for any other character
        _ => profile.default_width(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_string_returns_zero() {
        assert_eq!(estimate_title_pixel_width(""), 0);
        assert_eq!(estimate_description_pixel_width(""), 0);
    }

    #[test]
    fn test_short_title_under_limit() {
        // "Hello World" should be well under the 561px SERP title limit
        let width = estimate_title_pixel_width("Hello World");
        assert!(
            width < 561,
            "Short title 'Hello World' should be under 561px, got {width}"
        );
    }

    #[test]
    fn test_long_title_exceeds_limit() {
        // A very long title should exceed the 561px SERP title limit
        let long_title =
            "This Is A Very Long Title That Should Definitely Exceed The Google SERP Display Limit";
        let width = estimate_title_pixel_width(long_title);
        assert!(width > 561, "Long title should exceed 561px, got {width}");
    }

    #[test]
    fn test_short_description_under_limit() {
        // A short description should be well under the 985px SERP description limit
        let width = estimate_description_pixel_width("A short meta description.");
        assert!(
            width < 985,
            "Short description should be under 985px, got {width}"
        );
    }

    #[test]
    fn test_wide_chars_produce_larger_width() {
        let narrow = estimate_title_pixel_width("iiiiii");
        let wide = estimate_title_pixel_width("MWMWMW");
        assert!(
            wide > narrow,
            "Wide chars should produce larger width: wide={wide}, narrow={narrow}"
        );
    }

    #[test]
    fn test_description_smaller_than_title() {
        let text = "Same text for comparison";
        let title_width = estimate_title_pixel_width(text);
        let desc_width = estimate_description_pixel_width(text);
        assert!(
            title_width > desc_width,
            "Title width ({title_width}) should be larger than description width ({desc_width})"
        );
    }

    /// Regression: CJK characters fell through to the Latin default width, so a
    /// Chinese title was measured at roughly half its rendered width — and the
    /// character-count rules called a full-length title "too short".
    #[test]
    fn full_width_characters_are_wider_than_latin() {
        let cjk = estimate_title_pixel_width("维基百科");
        let latin = estimate_title_pixel_width("abcd");
        assert!(
            cjk > latin,
            "4 CJK chars ({cjk}px) should exceed 4 Latin chars ({latin}px)"
        );
    }

    #[test]
    fn full_width_text_is_detected_by_script_mix() {
        assert!(is_full_width_text("维基百科，自由的百科全书"));
        assert!(is_full_width_text("ウィキペディア"));
        assert!(is_full_width_text("위키백과"));
        assert!(!is_full_width_text("Welcome to GOV.UK"));
        // A mostly-Latin title with one CJK glyph is still Latin.
        assert!(!is_full_width_text("Wikipedia 维 the free encyclopedia"));
        assert!(!is_full_width_text(""));
    }
}
