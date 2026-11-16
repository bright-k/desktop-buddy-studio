use crate::{is_sequence_key, KeyStep, KeyboardLayout};
use unicode_normalization::UnicodeNormalization;

const INITIALS: [&str; 19] = [
    "r", "R", "s", "e", "E", "f", "a", "q", "Q", "t", "T", "d", "w", "W", "c", "z", "x", "v", "g",
];
const VOWELS: [&str; 21] = [
    "k", "o", "i", "O", "j", "p", "u", "P", "h", "hk", "ho", "hl", "y", "n", "nj", "np", "nl", "b",
    "m", "ml", "l",
];
const FINALS: [&str; 28] = [
    "", "r", "R", "rt", "s", "sw", "sg", "e", "f", "fr", "fa", "fq", "ft", "fx", "fv", "fg", "a",
    "q", "qt", "t", "T", "d", "w", "c", "z", "x", "v", "g",
];

pub fn compile_keyword(keyword: &str, layout: KeyboardLayout) -> Result<Vec<KeyStep>, String> {
    let normalized: String = keyword.nfc().collect();
    if !(1..=32).contains(&normalized.chars().count()) {
        return Err("keyword must contain 1..32 normalized characters".into());
    }
    let mut keys = String::new();
    for ch in normalized.chars() {
        if ('\u{ac00}'..='\u{d7a3}').contains(&ch) && layout == KeyboardLayout::Korean2Set {
            let value = (ch as u32 - 0xac00) as usize;
            keys.push_str(INITIALS[value / 588]);
            keys.push_str(VOWELS[(value % 588) / 28]);
            keys.push_str(FINALS[value % 28]);
        } else if ch.is_ascii_alphanumeric() || ch == ' ' {
            keys.push(ch);
        } else {
            return Err("keyword contains a character unsupported by the selected layout".into());
        }
    }
    let steps: Vec<_> = keys
        .chars()
        .map(|ch| KeyStep {
            code: if ch.is_ascii_alphabetic() {
                format!("Key{}", ch.to_ascii_uppercase())
            } else if ch.is_ascii_digit() {
                format!("Digit{ch}")
            } else {
                "Space".into()
            },
            shift: ch.is_ascii_uppercase(),
        })
        .collect();
    validate_steps(&steps)?;
    Ok(steps)
}
pub fn validate_steps(steps: &[KeyStep]) -> Result<(), String> {
    if !(1..=128).contains(&steps.len()) {
        return Err("sequence must contain 1..128 steps".into());
    }
    if steps.iter().any(|step| !is_sequence_key(&step.code)) {
        return Err("sequence contains an unsupported or reset key".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key_string(word: &str) -> String {
        compile_keyword(word, KeyboardLayout::Korean2Set)
            .unwrap()
            .iter()
            .map(|step| {
                let c = step.code.chars().last().unwrap();
                if step.shift {
                    c
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect()
    }
    #[test]
    fn korean_keywords_include_shift_compounds_and_finals() {
        assert_eq!(key_string("고양이"), "rhdiddl");
        assert_eq!(key_string("까꿰값"), "RkRnprkqt");
        assert_eq!(key_string("한글"), "gksrmf");
        assert_eq!(key_string("\u{1112}\u{1161}\u{11ab}글"), "gksrmf");
    }
    #[test]
    fn all_modern_syllables_produce_valid_steps() {
        for cp in 0xac00..=0xd7a3 {
            assert!(compile_keyword(
                &char::from_u32(cp).unwrap().to_string(),
                KeyboardLayout::Korean2Set
            )
            .is_ok());
        }
    }
    #[test]
    fn invalid_or_oversized_patterns_fail() {
        assert!(compile_keyword("고양이", KeyboardLayout::EnglishQwerty).is_err());
        assert!(compile_keyword("ㄱ", KeyboardLayout::Korean2Set).is_err());
        assert!(compile_keyword(&"괆".repeat(32), KeyboardLayout::Korean2Set).is_err());
        assert!(compile_keyword("", KeyboardLayout::EnglishQwerty).is_err());
    }
}
