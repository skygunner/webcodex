//! Closed Computer key/chord contract shared by schemas, admission and native input.

use crate::runner_protocol::RunnerCapabilityId;
use serde::Deserialize;

/// Navigation keys supported by the original key-input capability.

pub const COMPUTER_KEY_INPUT_KEYS: &[&str] = &[
    "enter",
    "escape",
    "tab",
    "arrow_up",
    "arrow_down",
    "arrow_left",
    "arrow_right",
    "page_up",
    "page_down",
    "home",
    "end",
];

/// Windows virtual-key shortcuts. Each requires exactly the Control modifier.
pub const COMPUTER_LETTER_SHORTCUT_KEYS: &[&str] = &["j", "p"];

const LETTER_KEYS: &[&str] = &[
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGIT_KEYS: &[&str] = &["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
const FUNCTION_KEYS: &[&str] = &[
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "f13", "f14", "f15",
    "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23", "f24",
];
const EDITING_KEYS: &[&str] = &["space", "backspace", "delete", "insert"];
const PUNCTUATION_KEYS: &[&str] = &[
    "semicolon",
    "equal",
    "comma",
    "minus",
    "period",
    "slash",
    "backquote",
    "left_bracket",
    "backslash",
    "right_bracket",
    "quote",
];
const NUMPAD_KEYS: &[&str] = &[
    "numpad_0",
    "numpad_1",
    "numpad_2",
    "numpad_3",
    "numpad_4",
    "numpad_5",
    "numpad_6",
    "numpad_7",
    "numpad_8",
    "numpad_9",
    "numpad_add",
    "numpad_subtract",
    "numpad_multiply",
    "numpad_divide",
    "numpad_decimal",
    "numpad_enter",
];

/// Logical virtual-key names, not Unicode text or physical scan-code positions.
pub fn computer_key_input_keys() -> impl Iterator<Item = &'static str> {
    [
        COMPUTER_KEY_INPUT_KEYS,
        LETTER_KEYS,
        DIGIT_KEYS,
        FUNCTION_KEYS,
        EDITING_KEYS,
        PUNCTUATION_KEYS,
        NUMPAD_KEYS,
    ]
    .into_iter()
    .flat_map(|keys| keys.iter().copied())
}

/// Canonical modifier order; platform adapters may reject unsupported chords.
pub const COMPUTER_KEY_INPUT_MODIFIERS: &[&str] = &["shift", "control", "option", "command"];

pub fn is_computer_letter_shortcut(key: &str) -> bool {
    COMPUTER_LETTER_SHORTCUT_KEYS.contains(&key)
}

pub fn validate_computer_key_input(key: &str, modifiers: &[String]) -> Result<(), &'static str> {
    if !computer_key_input_keys().any(|supported| supported == key) {
        return Err("computer key input key is outside the closed vocabulary");
    }
    validate_computer_key_modifiers(modifiers)
}

pub fn validate_computer_key_modifiers(modifiers: &[String]) -> Result<(), &'static str> {
    if modifiers.len() > COMPUTER_KEY_INPUT_MODIFIERS.len() {
        return Err("computer key input has too many modifiers");
    }
    for (index, modifier) in modifiers.iter().enumerate() {
        if !COMPUTER_KEY_INPUT_MODIFIERS.contains(&modifier.as_str())
            || modifiers[..index].contains(modifier)
        {
            return Err("computer key input modifiers are invalid or duplicated");
        }
    }
    Ok(())
}

pub fn computer_key_input_capabilities(
    key: &str,
    modifiers: &[String],
    general_chords: bool,
) -> &'static [RunnerCapabilityId] {
    if COMPUTER_KEY_INPUT_KEYS.contains(&key) {
        &[RunnerCapabilityId::ComputerKeyInput]
    } else if !general_chords && is_computer_letter_shortcut(key) && modifiers == ["control"] {
        &[
            RunnerCapabilityId::ComputerKeyInput,
            RunnerCapabilityId::ComputerLetterShortcuts,
        ]
    } else {
        &[
            RunnerCapabilityId::ComputerKeyInput,
            RunnerCapabilityId::ComputerKeyChords,
        ]
    }
}

/// Canonical bounded Runner payload; modifiers are always explicit on the wire.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerKeyInputPayload {
    pub surface_id: String,
    pub key: String,
    pub modifiers: Vec<String>,
}

impl ComputerKeyInputPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_computer_key_input(&self.key, &self.modifiers)
    }
}

pub fn computer_key_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let keys: Vec<_> = computer_key_input_keys().collect();
    schemars::json_schema!({
        "type": "string",
        "enum": keys,
    })
}

/// Preserve explicit null through the OpenAPI input-schema normalization path.
pub fn computer_key_modifiers_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "anyOf": [
            {
                "type": "array",
                "maxItems": COMPUTER_KEY_INPUT_MODIFIERS.len(),
                "uniqueItems": true,
                "items": {
                    "type": "string",
                    "enum": COMPUTER_KEY_INPUT_MODIFIERS,
                },
            },
            {"type": "null"}
        ]
    })
}

/// Additional effects on the existing Computer control operation are capability gated.
pub fn computer_control_capabilities(action: &str) -> &'static [RunnerCapabilityId] {
    if action == "select" {
        &[
            RunnerCapabilityId::ComputerControl,
            RunnerCapabilityId::ComputerSelection,
        ]
    } else {
        &[RunnerCapabilityId::ComputerControl]
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::runner_protocol::RunnerCapabilities;
    #[test]
    fn computer_selection_is_an_additive_capability() {
        let old: RunnerCapabilities = serde_json::from_str(r#"{"computer_control":true}"#).unwrap();
        assert!(!old.computer_selection);
        assert!(!RunnerCapabilities::default().computer_selection);
        let new: RunnerCapabilities =
            serde_json::from_str(r#"{"computer_selection":true}"#).unwrap();
        assert!(new.supports(RunnerCapabilityId::ComputerSelection));
        assert!(!new.computer_control);
        assert_eq!(
            computer_control_capabilities("select"),
            &[
                RunnerCapabilityId::ComputerControl,
                RunnerCapabilityId::ComputerSelection
            ]
        );
        assert_eq!(
            computer_control_capabilities("press"),
            &[RunnerCapabilityId::ComputerControl]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_key_chord_vocabulary_and_modifier_sets_are_bounded() {
        let keys: Vec<_> = computer_key_input_keys().collect();
        assert_eq!(keys.len(), 102);
        assert_eq!(
            keys.iter().collect::<std::collections::BTreeSet<_>>().len(),
            keys.len()
        );
        for key in &keys {
            for mask in 0..16 {
                let modifiers: Vec<_> = COMPUTER_KEY_INPUT_MODIFIERS
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, name)| (*name).to_string())
                    .collect();
                validate_computer_key_input(key, &modifiers).unwrap();
            }
        }
        for key in [
            "",
            "A",
            "Ctrl+P",
            "f0",
            "f01",
            "f25",
            "numpad_00",
            "windows",
            "a b",
        ] {
            assert!(validate_computer_key_input(key, &[]).is_err(), "{key}");
        }
        for modifiers in [
            vec!["ctrl".into()],
            vec!["control".into(), "control".into()],
            vec!["shift".into(); 5],
        ] {
            assert!(validate_computer_key_input("a", &modifiers).is_err());
        }
    }

    #[test]
    fn computer_key_chord_capabilities_classify_the_complete_chord() {
        use RunnerCapabilityId::{ComputerKeyChords, ComputerKeyInput, ComputerLetterShortcuts};
        assert_eq!(
            computer_key_input_capabilities("tab", &[], false),
            &[ComputerKeyInput]
        );
        for key in ["j", "p"] {
            assert_eq!(
                computer_key_input_capabilities(key, &["control".into()], false),
                &[ComputerKeyInput, ComputerLetterShortcuts]
            );
            assert_eq!(
                computer_key_input_capabilities(key, &["control".into()], true),
                &[ComputerKeyInput, ComputerKeyChords]
            );
            for modifiers in [
                vec![],
                vec!["shift".into()],
                vec!["control".into(), "shift".into()],
            ] {
                assert_eq!(
                    computer_key_input_capabilities(key, &modifiers, false),
                    &[ComputerKeyInput, ComputerKeyChords]
                );
            }
        }
        for key in ["a", "0", "f24", "slash", "numpad_enter", "delete"] {
            assert_eq!(
                computer_key_input_capabilities(key, &["control".into()], true),
                &[ComputerKeyInput, ComputerKeyChords]
            );
        }
    }
}
