use crate::config::SymbolReplacement;

/// Normalizes an utterance / spoken term for comparison: trims surrounding
/// whitespace, drops trailing sentence punctuation Whisper tends to append,
/// then lowercases. So "Pipe.", "PIPE" and " pipe " all normalize to "pipe".
fn normalize(s: &str) -> String {
    s.trim()
        .trim_end_matches(|c: char| matches!(c, '.' | ',' | '!' | '?' | ';' | ':'))
        .trim()
        .to_lowercase()
}

/// Returns the replacement character when the WHOLE utterance (normalized)
/// exactly matches an active entry's spoken term. Otherwise `None`, meaning the
/// text is left untouched. First match wins. Entries with an empty spoken term
/// or empty symbol are skipped.
pub fn resolve(text: &str, replacements: &[SymbolReplacement], enabled: bool) -> Option<String> {
    if !enabled {
        return None;
    }
    let key = normalize(text);
    if key.is_empty() {
        return None;
    }
    for r in replacements {
        if r.spoken.trim().is_empty() || r.symbol.is_empty() {
            continue;
        }
        if normalize(&r.spoken) == key {
            return Some(r.symbol.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Vec<SymbolReplacement> {
        vec![
            SymbolReplacement { spoken: "pipe".into(), symbol: "|".into() },
            SymbolReplacement { spoken: "backslash".into(), symbol: "\\".into() },
        ]
    }

    #[test]
    fn exact_match_returns_symbol() {
        assert_eq!(resolve("pipe", &table(), true), Some("|".into()));
    }

    #[test]
    fn normalizes_case_and_trailing_punctuation() {
        assert_eq!(resolve("Pipe.", &table(), true), Some("|".into()));
        assert_eq!(resolve("  PIPE ", &table(), true), Some("|".into()));
        assert_eq!(resolve("backslash?", &table(), true), Some("\\".into()));
    }

    #[test]
    fn sentence_containing_the_word_is_not_replaced() {
        assert_eq!(resolve("use a pipe here", &table(), true), None);
    }

    #[test]
    fn disabled_returns_none() {
        assert_eq!(resolve("pipe", &table(), false), None);
    }

    #[test]
    fn empty_table_returns_none() {
        assert_eq!(resolve("pipe", &[], true), None);
    }

    #[test]
    fn entries_with_blank_fields_are_skipped() {
        let t = vec![
            SymbolReplacement { spoken: "  ".into(), symbol: "|".into() },
            SymbolReplacement { spoken: "at".into(), symbol: "".into() },
        ];
        assert_eq!(resolve("at", &t, true), None);
    }

    #[test]
    fn first_match_wins() {
        let t = vec![
            SymbolReplacement { spoken: "pipe".into(), symbol: "|".into() },
            SymbolReplacement { spoken: "pipe".into(), symbol: "X".into() },
        ];
        assert_eq!(resolve("pipe", &t, true), Some("|".into()));
    }
}
