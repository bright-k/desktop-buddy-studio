pub const MODIFIERS: [&str; 8] = [
    "ShiftLeft",
    "ShiftRight",
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "MetaLeft",
    "MetaRight",
];
pub fn is_modifier(code: &str) -> bool {
    MODIFIERS.contains(&code)
}
pub fn is_supported_key(code: &str) -> bool {
    if code.len() == 4 && code.starts_with("Key") && code.as_bytes()[3].is_ascii_uppercase() {
        return true;
    }
    if code.len() == 6 && code.starts_with("Digit") && code.as_bytes()[5].is_ascii_digit() {
        return true;
    }
    if let Some(number) = code.strip_prefix('F') {
        if let Ok(value) = number.parse::<u8>() {
            if (1..=12).contains(&value) && number == value.to_string() {
                return true;
            }
        }
    }
    is_modifier(code)
        || [
            "Escape",
            "Tab",
            "Space",
            "Enter",
            "Backspace",
            "Delete",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "ArrowLeft",
            "ArrowRight",
            "ArrowUp",
            "ArrowDown",
            "Backquote",
            "Minus",
            "Equal",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "Semicolon",
            "Quote",
            "Comma",
            "Period",
            "Slash",
        ]
        .contains(&code)
}
pub fn is_sequence_key(code: &str) -> bool {
    is_supported_key(code) && !is_modifier(code) && !["Backspace", "Delete"].contains(&code)
}
