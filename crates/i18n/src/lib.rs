use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, sync::OnceLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "ja-JP")]
    Japanese,
    #[serde(rename = "en-US")]
    English,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::Chinese => "zh-CN",
            Self::Japanese => "ja-JP",
            Self::English => "en-US",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "zh-CN" => Some(Self::Chinese),
            "ja-JP" => Some(Self::Japanese),
            "en-US" => Some(Self::English),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct Translation {
    #[serde(rename = "zh-CN")]
    chinese: String,
    #[serde(rename = "ja-JP")]
    japanese: String,
    #[serde(rename = "en-US")]
    english: String,
}

fn catalog() -> &'static BTreeMap<String, Translation> {
    static CATALOG: OnceLock<BTreeMap<String, Translation>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../locales.json")).expect("Invalid translation catalog")
    })
}

pub fn tr(language: Language, key: &str) -> &'static str {
    let entry = catalog()
        .get(key)
        .unwrap_or_else(|| panic!("Missing translation: {key}"));
    match language {
        Language::Chinese => &entry.chinese,
        Language::Japanese => &entry.japanese,
        Language::English => &entry.english,
    }
}

pub fn format_text(language: Language, key: &str, arguments: &[(&str, &str)]) -> String {
    let mut text = tr(language, key).to_owned();
    for (name, value) in arguments {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// A language-independent notice/error, translated when it is displayed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub key: String,
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

impl Message {
    pub fn new(key: &str) -> Self {
        Self {
            key: key.into(),
            args: BTreeMap::new(),
        }
    }

    pub fn arg(mut self, name: &str, value: impl ToString) -> Self {
        self.args.insert(name.into(), value.to_string());
        self
    }

    pub fn detail(key: &str, value: impl ToString) -> Self {
        Self::new(key).arg("detail", value)
    }

    pub fn render(&self, language: Language) -> String {
        if !catalog().contains_key(&self.key) {
            return format_text(language, "error.operation", &[("detail", &self.key)]);
        }
        let args: Vec<_> = self
            .args
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        format_text(language, &self.key, &args)
    }

    pub fn from_error(payload: &str) -> Self {
        serde_json::from_str(payload).unwrap_or_else(|_| Self::detail("error.operation", payload))
    }
}

impl fmt::Display for Message {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.render(Language::default()))
    }
}

impl std::error::Error for Message {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|p| p.0))
            .collect()
    }

    #[test]
    fn every_translation_is_nonempty_and_has_matching_placeholders() {
        for (key, entry) in catalog() {
            assert!(!entry.chinese.trim().is_empty(), "{key}");
            assert!(!entry.japanese.trim().is_empty(), "{key}");
            assert!(!entry.english.trim().is_empty(), "{key}");
            assert_eq!(
                placeholders(&entry.chinese),
                placeholders(&entry.japanese),
                "{key}"
            );
            assert_eq!(
                placeholders(&entry.chinese),
                placeholders(&entry.english),
                "{key}"
            );
        }
    }

    #[test]
    fn structured_messages_translate_after_serialization() {
        let message = Message::new("notice.scan_complete").arg("count", 9);
        let restored = Message::from_error(&serde_json::to_string(&message).unwrap());
        assert!(restored.render(Language::English).contains("9"));
        assert!(restored.render(Language::Japanese).contains("スキャン"));
        assert_eq!(Language::from_code("de-DE"), None);
    }
}
