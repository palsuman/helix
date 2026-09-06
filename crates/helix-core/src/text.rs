//! Unicode-correct text primitives shared by future editor implementations.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnicodeWarningKind {
    BidiControl,
    Invisible,
    Confusable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeWarning {
    /// UTF-8 byte range in the original text.
    pub start: usize,
    pub end: usize,
    pub character: char,
    pub kind: UnicodeWarningKind,
    pub message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnicodeWarningDecoration {
    /// UTF-8 byte range; editor adapters convert it to their native position model.
    pub start: usize,
    pub end: usize,
    pub class_name: &'static str,
    pub hover_message: &'static str,
}

pub fn next_grapheme_boundary(text: &str, byte_offset: usize) -> usize {
    let offset = floor_char_boundary(text, byte_offset);
    if offset >= text.len() {
        return text.len();
    }
    text[offset..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(text.len(), |(relative, _)| offset + relative)
}

pub fn previous_grapheme_boundary(text: &str, byte_offset: usize) -> usize {
    let offset = floor_char_boundary(text, byte_offset);
    if offset == 0 {
        return 0;
    }
    text[..offset]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

pub fn delete_previous_grapheme(text: &mut String, byte_offset: usize) -> usize {
    let end = floor_char_boundary(text, byte_offset);
    let start = previous_grapheme_boundary(text, end);
    text.replace_range(start..end, "");
    start
}

pub fn delete_next_grapheme(text: &mut String, byte_offset: usize) -> usize {
    let start = floor_char_boundary(text, byte_offset);
    let end = next_grapheme_boundary(text, start);
    text.replace_range(start..end, "");
    start
}

pub fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn display_width_cjk(text: &str) -> usize {
    UnicodeWidthStr::width_cjk(text)
}

pub fn unicode_warnings(text: &str) -> Vec<UnicodeWarning> {
    text.char_indices()
        .filter_map(|(start, character)| {
            let warning = warning_for(character, start == 0)?;
            Some(UnicodeWarning {
                start,
                end: start + character.len_utf8(),
                character,
                kind: warning.0,
                message: warning.1,
            })
        })
        .collect()
}

pub fn unicode_warning_decorations(text: &str) -> Vec<UnicodeWarningDecoration> {
    unicode_warnings(text)
        .into_iter()
        .map(|warning| UnicodeWarningDecoration {
            start: warning.start,
            end: warning.end,
            class_name: match warning.kind {
                UnicodeWarningKind::BidiControl => "unicode-warning-bidi-control",
                UnicodeWarningKind::Invisible => "unicode-warning-invisible",
                UnicodeWarningKind::Confusable => "unicode-warning-confusable",
            },
            hover_message: warning.message,
        })
        .collect()
}

fn floor_char_boundary(text: &str, requested: usize) -> usize {
    let mut offset = requested.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn warning_for(character: char, at_start: bool) -> Option<(UnicodeWarningKind, &'static str)> {
    if is_bidi_control(character) {
        return Some((
            UnicodeWarningKind::BidiControl,
            "bidirectional control character can change source-code display order",
        ));
    }
    if is_suspicious_invisible(character, at_start) {
        return Some((
            UnicodeWarningKind::Invisible,
            "invisible Unicode character can conceal or split source tokens",
        ));
    }
    if is_common_confusable(character) {
        return Some((
            UnicodeWarningKind::Confusable,
            "Unicode character is visually confusable with a common source-code character",
        ));
    }
    None
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn is_suspicious_invisible(character: char, at_start: bool) -> bool {
    matches!(
        character,
        '\u{00ad}'
            | '\u{034f}'
            | '\u{115f}'
            | '\u{1160}'
            | '\u{17b4}'
            | '\u{17b5}'
            | '\u{180e}'
            | '\u{200b}'
            | '\u{2060}'
            | '\u{3164}'
            | '\u{ffa0}'
    ) || (character == '\u{feff}' && !at_start)
}

fn is_common_confusable(character: char) -> bool {
    matches!(
        character,
        // Cyrillic characters commonly substituted into Latin identifiers.
        '\u{0405}'
            | '\u{0410}'
            | '\u{0412}'
            | '\u{0415}'
            | '\u{041a}'
            | '\u{041c}'
            | '\u{041d}'
            | '\u{041e}'
            | '\u{0420}'
            | '\u{0421}'
            | '\u{0422}'
            | '\u{0425}'
            | '\u{0430}'
            | '\u{0435}'
            | '\u{043e}'
            | '\u{0440}'
            | '\u{0441}'
            | '\u{0445}'
            | '\u{0455}'
            // Greek characters commonly substituted into Latin identifiers.
            | '\u{0391}'
            | '\u{0392}'
            | '\u{0395}'
            | '\u{0396}'
            | '\u{0397}'
            | '\u{0399}'
            | '\u{039a}'
            | '\u{039c}'
            | '\u{039d}'
            | '\u{039f}'
            | '\u{03a1}'
            | '\u{03a4}'
            | '\u{03a5}'
            | '\u{03a7}'
            | '\u{03bf}'
            | '\u{03c1}'
            | '\u{03c7}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_and_deletes_by_extended_grapheme_cluster() {
        let text = "a\u{301}👨‍👩‍👧‍👦🇺🇳👍🏽界";
        let boundaries: Vec<_> = text
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain(std::iter::once(text.len()))
            .collect();
        let mut offset = 0;
        for expected in boundaries.iter().skip(1) {
            offset = next_grapheme_boundary(text, offset);
            assert_eq!(&offset, expected);
        }
        for expected in boundaries.iter().rev().skip(1) {
            offset = previous_grapheme_boundary(text, offset);
            assert_eq!(&offset, expected);
        }

        let mut editable = text.to_string();
        let family_start = "a\u{301}".len();
        assert_eq!(
            delete_next_grapheme(&mut editable, family_start),
            family_start
        );
        assert_eq!(editable, "a\u{301}🇺🇳👍🏽界");
        let cursor = "a\u{301}🇺🇳👍🏽".len();
        assert_eq!(
            delete_previous_grapheme(&mut editable, cursor),
            "a\u{301}🇺🇳".len()
        );
        assert_eq!(editable, "a\u{301}🇺🇳界");
    }

    #[test]
    fn reports_terminal_and_cjk_display_width_without_counting_combining_marks() {
        assert_eq!(display_width("a\u{301}"), 1);
        assert_eq!(display_width("界"), 2);
        assert_eq!(display_width_cjk("·"), 2);
        assert_eq!(display_width("👨‍👩‍👧‍👦"), 2);
    }

    #[test]
    fn detects_bidi_controls_invisibles_and_confusables_with_byte_ranges() {
        let text = "ok\u{202e}safe\u{200b}pаypal";
        let warnings = unicode_warnings(text);
        assert_eq!(warnings.len(), 3);
        assert_eq!(warnings[0].kind, UnicodeWarningKind::BidiControl);
        assert_eq!(&text[warnings[0].start..warnings[0].end], "\u{202e}");
        assert_eq!(warnings[1].kind, UnicodeWarningKind::Invisible);
        assert_eq!(warnings[2].kind, UnicodeWarningKind::Confusable);
        assert_eq!(warnings[2].character, 'а');

        let decorations = unicode_warning_decorations(text);
        assert_eq!(decorations[0].class_name, "unicode-warning-bidi-control");
        assert_eq!(decorations[1].class_name, "unicode-warning-invisible");
        assert_eq!(decorations[2].class_name, "unicode-warning-confusable");
        assert_eq!(
            decorations[0].start..decorations[0].end,
            warnings[0].start..warnings[0].end
        );
    }

    #[test]
    fn permits_emoji_joiners_script_joiners_and_a_leading_bom() {
        assert!(unicode_warnings("\u{feff}👩‍💻क्‍ष").is_empty());
        let warnings = unicode_warnings("x\u{feff}y");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].kind, UnicodeWarningKind::Invisible);
    }
}
