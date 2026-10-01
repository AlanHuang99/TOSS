//! Validated account display names.

use thiserror::Error;

const MAX_DISPLAY_NAME_CHARS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DisplayName(String);

impl DisplayName {
    pub(crate) fn parse(raw: &str) -> Result<Self, InvalidDisplayName> {
        let value = raw.trim();
        if value.is_empty()
            || value.chars().count() > MAX_DISPLAY_NAME_CHARS
            || value.chars().any(char::is_control)
        {
            return Err(InvalidDisplayName);
        }
        Ok(Self(value.to_string()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("display name is invalid")]
pub(crate) struct InvalidDisplayName;

#[cfg(test)]
mod tests {
    use super::{DisplayName, InvalidDisplayName, MAX_DISPLAY_NAME_CHARS};

    #[test]
    fn display_names_are_trimmed() -> Result<(), InvalidDisplayName> {
        assert_eq!(
            DisplayName::parse("  Ada Lovelace ")?.as_str(),
            "Ada Lovelace"
        );
        Ok(())
    }

    #[test]
    fn empty_control_and_oversized_display_names_are_rejected() {
        assert_eq!(DisplayName::parse(""), Err(InvalidDisplayName));
        assert_eq!(DisplayName::parse(" \t "), Err(InvalidDisplayName));
        assert_eq!(DisplayName::parse("Ada\nLovelace"), Err(InvalidDisplayName));
        assert_eq!(DisplayName::parse("Ada\u{7}"), Err(InvalidDisplayName));
        assert_eq!(
            DisplayName::parse(&"a".repeat(MAX_DISPLAY_NAME_CHARS + 1)),
            Err(InvalidDisplayName)
        );
    }

    #[test]
    fn maximum_length_is_measured_in_unicode_characters() -> Result<(), InvalidDisplayName> {
        let value = "界".repeat(MAX_DISPLAY_NAME_CHARS);
        assert_eq!(DisplayName::parse(&value)?.as_str(), value);
        Ok(())
    }
}
