//! Field validation and conversion between Vita UTF-16 and Nivora UTF-8 editing.
use crate::config::Config;
use anyhow::{Result, ensure};
use nivora_platform::TextInputEvent;
use nivora_ui::TextInputState;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebDavField {
    Url,
    User,
    Password,
}
impl WebDavField {
    pub fn key(self) -> &'static str {
        match self {
            Self::Url => "url",
            Self::User => "user",
            Self::Password => "password",
        }
    }
    pub fn limit(self) -> usize {
        match self {
            Self::Url => 2048,
            Self::User | Self::Password => 256,
        }
    }
    pub fn value(self, config: &Config) -> &str {
        match self {
            Self::Url => &config.webdav_url,
            Self::User => &config.webdav_user,
            Self::Password => &config.webdav_password,
        }
    }
    pub fn apply(self, config: &Config, value: String) -> Result<Config> {
        validate_text(&value, self.limit())?;
        let mut next = config.clone();
        match self {
            Self::Url => {
                let value = value.trim();
                if !value.is_empty() {
                    let url = url::Url::parse(value)
                        .map_err(|_| anyhow::anyhow!("Invalid WebDAV URL"))?;
                    ensure!(
                        ["http", "https"].contains(&url.scheme()) && url.host_str().is_some(),
                        "WebDAV needs an HTTP(S) URL"
                    );
                    ensure!(
                        url.username().is_empty()
                            && url.password().is_none()
                            && url.query().is_none()
                            && url.fragment().is_none(),
                        "Put credentials in their own fields; URL must not contain query or fragment"
                    );
                }
                next.webdav_url = value.into();
            }
            Self::User => {
                ensure!(!value.contains(':'), "WebDAV username cannot contain ':'");
                next.webdav_user = value;
            }
            Self::Password => next.webdav_password = value,
        }
        Ok(next)
    }
}
pub fn validate_text(value: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.chars().any(char::is_control),
        "Input contains control characters"
    );
    ensure!(
        value.encode_utf16().count() <= limit,
        "Input exceeds {limit} UTF-16 units"
    );
    Ok(())
}

/// Read the terminated committed buffer, independently of display/preedit events.
pub fn committed_utf16(input: &[u16]) -> Result<String> {
    let length = input
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| anyhow::anyhow!("Unterminated system keyboard text"))?;
    Ok(String::from_utf16(&input[..length])?)
}

// No Debug implementation: this state can contain credentials.
pub struct Edit {
    pub field: WebDavField,
    pub text: EditText,
}
impl Edit {
    pub fn new(field: WebDavField, config: &Config) -> Result<Self> {
        let value = field.value(config);
        validate_text(value, field.limit())?;
        Ok(Self {
            field,
            text: EditText::new(value.to_owned()),
        })
    }
}
pub struct EditText {
    pub value: String,
    pub caret: usize,
    pub preedit: Option<Range<usize>>,
}
impl EditText {
    pub fn new(value: String) -> Self {
        Self {
            caret: value.len(),
            value,
            preedit: None,
        }
    }
    pub fn from_utf16(text: &[u16], caret: usize, preedit: Range<usize>) -> Result<Self> {
        let value = String::from_utf16(text)?;
        let offset = |index: usize| -> Result<usize> {
            ensure!(index <= text.len(), "Invalid IME position");
            Ok(String::from_utf16(&text[..index])?.len())
        };
        let caret = offset(caret)?;
        let preedit = if preedit.is_empty() {
            None
        } else {
            ensure!(preedit.start < preedit.end, "Invalid IME composition range");
            Some(offset(preedit.start)?..offset(preedit.end)?)
        };
        Ok(Self {
            value,
            caret,
            preedit,
        })
    }
    pub fn ui_state(&self, password: bool) -> TextInputState {
        let display = |s: &str| {
            if password {
                "•".repeat(s.chars().count())
            } else {
                s.into()
            }
        };
        if let Some(range) = &self.preedit {
            let prefix = display(&self.value[..range.start]);
            let preedit = display(&self.value[range.clone()]);
            let suffix = display(&self.value[range.end..]);
            let cursor =
                display(&self.value[range.start..self.caret.clamp(range.start, range.end)]).len();
            let mut state = TextInputState::new(format!("{prefix}{suffix}"));
            select_at(&mut state, prefix.len());
            state.handle(TextInputEvent::Preedit {
                text: preedit,
                cursor: Some((cursor, cursor)),
            });
            state
        } else {
            let mut state = TextInputState::new(display(&self.value));
            select_at(&mut state, display(&self.value[..self.caret]).len());
            state
        }
    }
}
fn select_at(state: &mut TextInputState, mut index: usize) {
    // Vita positions count UTF-16 units; Nivora caret positions are grapheme boundaries.
    while !state.set_selection(index, index) && index > 0 {
        index -= 1;
    }
}
