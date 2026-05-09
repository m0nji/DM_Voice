const BARE_KEYS: &[&str] = &["MetaRight", "MetaLeft"];

pub fn is_valid_shortcut(shortcut: &str) -> bool {
    if BARE_KEYS.contains(&shortcut) {
        return true;
    }
    let parts: Vec<&str> = shortcut.split('+').collect();
    let modifiers = ["Alt", "Ctrl", "Super", "Shift"];
    let has_modifier = parts.iter().any(|p| modifiers.contains(p));
    let has_key = parts.iter().any(|p| !modifiers.contains(p) && !p.is_empty());
    has_modifier && has_key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_space_is_valid() {
        assert!(is_valid_shortcut("Alt+Space"));
    }

    #[test]
    fn meta_right_is_valid() {
        assert!(is_valid_shortcut("MetaRight"));
    }

    #[test]
    fn bare_key_is_invalid() {
        assert!(!is_valid_shortcut("Space"));
    }

    #[test]
    fn modifier_only_is_invalid() {
        assert!(!is_valid_shortcut("Alt"));
    }
}
