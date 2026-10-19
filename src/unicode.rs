//! UTF-16 input offsets and Unicode-safe deletion, independent of UIKit.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Convert a UTF-16 range into a UTF-8 range, expanding split surrogate pairs.
pub fn utf16_range(text: &str, range: Range<usize>) -> Range<usize> {
    let mut start = text.len();
    let mut end = text.len();
    let mut units = 0;
    for (byte, character) in text.char_indices() {
        let next = units + character.len_utf16();
        if units <= range.start && range.start < next {
            start = byte;
        }
        if units < range.end && range.end <= next {
            end = byte + character.len_utf8();
        }
        units = next;
    }
    if range.start == range.end {
        end = start;
    }
    start..end.max(start)
}

/// The complete grapheme before a UTF-16 caret. Empty at the document start.
pub fn previous_grapheme(text: &str, caret: usize) -> Range<usize> {
    let mut units = 0;
    let mut previous = 0..0;
    for grapheme in text.graphemes(true) {
        let next = units + grapheme.encode_utf16().count();
        if units >= caret {
            break;
        }
        previous = units..next;
        units = next;
    }
    previous
}

/// Clamp a UTF-16 selection to valid scalar boundaries, preserving empty carets.
pub fn normalized_range(text: &str, range: Range<usize>) -> Range<usize> {
    let bytes = utf16_range(text, range);
    text[..bytes.start].encode_utf16().count()..text[..bytes.end].encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surrogate_and_mixed_script_ranges_round_trip() {
        let text = "س😀Aب";
        assert_eq!(&text[utf16_range(text, 1..3)], "😀");
        assert_eq!(&text[utf16_range(text, 2..3)], "😀");
        assert_eq!(normalized_range(text, 2..3), 1..3);
        assert_eq!(normalized_range(text, 2..2), 1..1);
        assert_eq!(&text[utf16_range(text, 4..5)], "ب");
    }

    #[test]
    fn backspace_never_splits_arabic_marks_or_emoji_families() {
        for grapheme in ["بِ", "👨‍👩‍👧‍👦", "🇩🇿", "e\u{301}"] {
            let text = format!("a{grapheme}");
            let end = text.encode_utf16().count();
            let range = previous_grapheme(&text, end);
            assert_eq!(&text[utf16_range(&text, range)], grapheme);
        }
        assert_eq!(previous_grapheme("العربية", 0), 0..0);
    }
}
