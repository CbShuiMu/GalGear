//! Bounded native-state classification, never execution or dialogue-node validation.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{fs, io::Read, path::Path};

const MAX_INPUT: u64 = 8 * 1024 * 1024;
const MAX_TEXT: usize = 16 * 1024 * 1024;

/// File identity inferred from a complete recognized native data record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SaveKind {
    /// A native save state with execution context; its current node is unverified.
    Progress,
    /// Recognized options or global unlock/read flags without execution state.
    Control,
    /// Unrecognized, incomplete or ambiguous data; retain it for model inspection.
    Unknown,
}

/// Classify KiriKiri data independently of its filename or extension.
pub fn classify(path: &Path) -> SaveKind {
    read_document(path).map_or(SaveKind::Unknown, |value| classify_value(&value))
}

pub(super) fn recognizable(path: &Path) -> bool {
    read_document(path).is_some_and(|value| value.is_object())
}

fn read_document(path: &Path) -> Option<Value> {
    Parser::new(&read_text(path)?).parse()
}

fn read_text(path: &Path) -> Option<String> {
    text_stream(&read_bytes(path)?)
}

fn read_bytes(path: &Path) -> Option<Vec<u8>> {
    fs::File::open(path).ok().and_then(|file| {
        if file.metadata().ok()?.len() > MAX_INPUT {
            return None;
        }
        let mut bytes = Vec::new();
        file.take(MAX_INPUT + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= MAX_INPUT).then_some(bytes)
    })
}

/// Automatic backup comparison only. Keep every value and array position, but
/// disregard dictionary serialization order and repeated visits to boot screens.
/// Unknown files, BMP slots, malformed data and unfamiliar counters stay strict.
pub(crate) fn backup_identity(path: &Path) -> Option<Value> {
    if !path.extension()?.eq_ignore_ascii_case("ksd") {
        return None;
    }
    let data = read_bytes(path)?;
    if data.starts_with(b"BM") {
        return None;
    }
    let text = text_stream(&data)?;
    let mut parser = Parser::new(&text);
    parser.lossless = true;
    let mut value = parser.parse()?;
    if Parser::new(&text)
        .parse()
        .is_none_or(|value| classify_value(&value) != SaveKind::Control)
    {
        return value.is_object().then_some(value);
    }
    let fields = value.as_object_mut()?;
    // These KAG labels are startup/title/exit bookkeeping, not story read flags.
    // Keep zero, new keys, negative values and every other trail counter intact.
    for key in [
        "trail_custom_",
        "trail_custom_logo",
        "trail_custom_exit_begin",
        "trail_custom_exit_end",
        "trail_first_",
        "trail_first_common_macro",
        "trail_first_first",
        "trail_end_exit_begin",
        "trail_version_",
    ] {
        if let Some(counter) = fields.get_mut(key)
            && counter
                .as_str()
                .and_then(|token| token.strip_prefix("number:"))
                .and_then(|token| token.parse::<u64>().ok())
                .is_some_and(|count| count > 0)
        {
            *counter = Value::String("number:1".into());
        }
    }
    Some(value)
}

fn text_stream(data: &[u8]) -> Option<String> {
    let record = if data.starts_with(b"BM") {
        let size = u32::from_le_bytes(data.get(2..6)?.try_into().ok()?) as usize;
        if size < 14 {
            return None;
        }
        data.get(size..)?
    } else {
        data
    };
    let bytes = if record.starts_with(&[0xfe, 0xfe, 2, 0xff, 0xfe]) {
        let packed = u64::from_le_bytes(record.get(5..13)?.try_into().ok()?);
        let unpacked = u64::from_le_bytes(record.get(13..21)?.try_into().ok()?);
        let payload = record.get(21..)?;
        if packed != payload.len() as u64 || unpacked == 0 || unpacked > MAX_TEXT as u64 {
            return None;
        }
        let mut decoder = flate2::read::ZlibDecoder::new(payload);
        let mut result = Vec::new();
        (&mut decoder)
            .take(unpacked + 1)
            .read_to_end(&mut result)
            .ok()?;
        if result.len() as u64 != unpacked || decoder.total_in() != packed {
            return None;
        }
        result
    } else if record.starts_with(&[0xfe, 0xfe, 1, 0xff, 0xfe]) {
        record[5..]
            .iter()
            .map(|byte| ((byte & 0xaa) >> 1) | ((byte & 0x55) << 1))
            .collect()
    } else if record.starts_with(&[0xff, 0xfe]) {
        record[2..].to_vec()
    } else {
        let text = std::str::from_utf8(record)
            .ok()?
            .trim_start_matches('\u{feff}');
        return (text.len() <= MAX_TEXT).then(|| text.to_owned());
    };
    if bytes.len() > MAX_TEXT || bytes.len() % 2 != 0 {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
fn classify_bytes(data: &[u8]) -> SaveKind {
    let Some(text) = text_stream(data) else {
        return SaveKind::Unknown;
    };
    let Some(value) = Parser::new(&text).parse() else {
        return SaveKind::Unknown;
    };
    classify_value(&value)
}

fn classify_value(value: &Value) -> SaveKind {
    if has_execution(value) {
        return SaveKind::Progress;
    }
    let Some(fields) = value.as_object() else {
        return SaveKind::Unknown;
    };
    // Options may contain volume maps, and unlock state may contain read-label
    // maps. Preserve unfamiliar execution structures even beside global flags.
    let controls = fields
        .keys()
        .filter(|key| {
            matches!(
                key.to_ascii_lowercase().as_str(),
                "fullscreen"
                    | "windowwidth"
                    | "windowheight"
                    | "windowleft"
                    | "windowtop"
                    | "languagetype"
                    | "uilanguagetype"
                    | "automodepagewait"
                    | "automodelinewait"
                    | "automodewaitvoice"
                    | "bgmvolume"
                    | "sevolume"
                    | "voicevolume"
            ) || ["trail_", "autotrail_", "cg_", "bgm_", "voice_"]
                .iter()
                .any(|prefix| key.starts_with(prefix))
        })
        .count();
    if !has_execution_hint(value) && controls >= 2 {
        SaveKind::Control
    } else {
        SaveKind::Unknown
    }
}

fn has_execution(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            let storage = fields
                .get("currentStorage")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.trim().is_empty());
            let history = fields
                .get("historyData")
                .and_then(Value::as_object)
                .and_then(|history| history.get("data"))
                .and_then(Value::as_array)
                .is_some_and(|rows| !rows.is_empty());
            // A conductor also exists in title/menu initialization. Recorded
            // story history is a stronger candidate signal, never node proof.
            (storage && history) || fields.values().any(has_execution)
        }
        Value::Array(items) => items.iter().any(has_execution),
        _ => false,
    }
}

fn has_execution_hint(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.keys().any(|key| {
                matches!(
                    key.to_ascii_lowercase().as_str(),
                    "currentstorage"
                        | "currentlabel"
                        | "currentpage"
                        | "currentpagename"
                        | "currentnum"
                        | "currentline"
                        | "currentposition"
                        | "mainconductor"
                        | "conductor"
                        | "sceneplayer"
                        | "historydata"
                        | "scene"
                        | "chapter"
                        | "source"
                        | "line"
                        | "position"
                        | "label"
                        | "storagename"
                        | "storageshortname"
                        | "curlabel"
                        | "curline"
                        | "curpos"
                        | "scriptname"
                        | "scenarioname"
                        | "bookmark"
                        | "bookmarkinfo"
                        | "resumenode"
                        | "currentnode"
                )
            }) || fields.values().any(has_execution_hint)
        }
        Value::Array(items) => items.iter().any(has_execution_hint),
        Value::String(text) => {
            let text = text.trim();
            // Unknown wrappers can store a KAG conductor directly, without the
            // standard core/mainConductor keys. A native label/storage-shaped
            // value is enough to preserve uncertainty, never to infer progress.
            text.starts_with('*') && text.len() > 1
                || (text.to_ascii_lowercase().ends_with(".ks")
                    && !text.contains(['\r', '\n', '\0']))
        }
        _ => false,
    }
}

// Fixed data grammar only: no evaluator, identifiers, calls or model-authored
// parsing rules. Full consumption prevents string snippets from proving identity.
struct Parser<'a> {
    text: &'a str,
    at: usize,
    nodes: usize,
    lossless: bool,
}
impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            at: 0,
            nodes: 0,
            lossless: false,
        }
    }
    fn rest(&self) -> &'a str {
        &self.text[self.at..]
    }
    fn whitespace(&mut self) -> Option<()> {
        loop {
            while self.rest().chars().next().is_some_and(char::is_whitespace) {
                self.at += self.rest().chars().next()?.len_utf8();
            }
            if self.rest().starts_with("//") {
                self.at += self.rest().find('\n').unwrap_or(self.rest().len());
            } else if self.rest().starts_with("/*") {
                self.at += 2;
                let mut depth = 1;
                while depth > 0 {
                    if self.rest().starts_with("/*") {
                        depth += 1;
                        self.at += 2;
                    } else if self.rest().starts_with("*/") {
                        depth -= 1;
                        self.at += 2;
                    } else {
                        self.at += self.rest().chars().next()?.len_utf8();
                    }
                    if depth > 32 {
                        return None;
                    }
                }
            } else {
                return Some(());
            }
        }
    }
    fn consume(&mut self, value: &str) -> bool {
        if self.rest().starts_with(value) {
            self.at += value.len();
            true
        } else {
            false
        }
    }
    fn string(&mut self) -> Option<String> {
        if !self.consume("\"") {
            return None;
        }
        let mut units = Vec::new();
        loop {
            let character = self.rest().chars().next()?;
            self.at += character.len_utf8();
            if character == '"' {
                self.whitespace()?;
                if self.consume("\"") {
                    continue;
                }
                return String::from_utf16(&units).ok();
            }
            if character == '\\' {
                let escaped = self.rest().chars().next()?;
                self.at += escaped.len_utf8();
                let character = match escaped {
                    '"' => '"',
                    '\'' => '\'',
                    '\\' => '\\',
                    'n' => '\n',
                    'r' => '\r',
                    't' => '\t',
                    'b' => '\u{8}',
                    'f' => '\u{c}',
                    'v' => '\u{b}',
                    'a' => '\u{7}',
                    '0' => '\0',
                    'x' | 'X' => {
                        let start = self.at;
                        for _ in 0..4 {
                            if self
                                .rest()
                                .as_bytes()
                                .first()
                                .is_some_and(u8::is_ascii_hexdigit)
                            {
                                self.at += 1;
                            } else {
                                break;
                            }
                        }
                        if start == self.at {
                            return None;
                        }
                        units.push(u16::from_str_radix(&self.text[start..self.at], 16).ok()?);
                        continue;
                    }
                    _ => return None,
                };
                units.extend_from_slice(character.encode_utf16(&mut [0; 2]));
            } else {
                if character.is_control() {
                    return None;
                }
                units.extend_from_slice(character.encode_utf16(&mut [0; 2]));
            }
        }
    }
    fn value(&mut self, depth: usize) -> Option<Value> {
        self.nodes += 1;
        if depth > 64 || self.nodes > 200_000 {
            return None;
        }
        self.whitespace()?;
        let constant = self.consume("(const)");
        self.whitespace()?;
        if self.consume("%[") {
            let mut object = Map::new();
            self.whitespace()?;
            if self.consume("]") {
                return Some(Value::Object(object));
            }
            loop {
                self.whitespace()?;
                let key = self.string()?;
                self.whitespace()?;
                if !self.consume("=>") {
                    return None;
                }
                if object.insert(key, self.value(depth + 1)?).is_some() {
                    return None;
                }
                self.whitespace()?;
                if self.consume("]") {
                    return Some(Value::Object(object));
                }
                if !self.consume(",") {
                    return None;
                }
            }
        }
        if self.consume("[") {
            let mut array = Vec::new();
            self.whitespace()?;
            if self.consume("]") {
                return Some(Value::Array(array));
            }
            loop {
                array.push(self.value(depth + 1)?);
                self.whitespace()?;
                if self.consume("]") {
                    return Some(Value::Array(array));
                }
                if !self.consume(",") {
                    return None;
                }
            }
        }
        if constant {
            return None;
        }
        if self.rest().starts_with('"') {
            return self.string().map(|text| {
                Value::String(if self.lossless {
                    format!("string:{text}")
                } else {
                    text
                })
            });
        }
        for literal in ["void", "null"] {
            if self.consume(literal) {
                return Some(if self.lossless {
                    Value::String(format!("literal:{literal}"))
                } else {
                    Value::Null
                });
            }
        }
        let start = self.at;
        while self
            .rest()
            .as_bytes()
            .first()
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !b",]".contains(byte))
        {
            self.at += 1;
            if !self.text.is_char_boundary(self.at) {
                return None;
            }
        }
        let token = &self.text[start..self.at];
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        // TJS writes finite reals as hexadecimal floats. They are scalar state,
        // never dialogue coordinates here; validate their syntax without evaluation.
        static NUMBER: std::sync::OnceLock<Option<regex::Regex>> = std::sync::OnceLock::new();
        let expression = NUMBER.get_or_init(|| regex::Regex::new(
            r"^-?(?:0[xX][0-9a-fA-F]+(?:\.[0-9a-fA-F]*)?[pP][+-]?[0-9]+|(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?)$").ok()).as_ref()?;
        expression.is_match(token).then(|| {
            if self.lossless {
                Value::String(format!("number:{token}"))
            } else {
                Value::Null
            }
        })
    }
    fn parse(mut self) -> Option<Value> {
        let value = self.value(0)?;
        self.whitespace()?;
        (self.at == self.text.len()).then_some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn compressed(text: &str) -> Vec<u8> {
        let raw = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&raw).unwrap();
        let packed = encoder.finish().unwrap();
        let mut bytes = vec![0xfe, 0xfe, 2, 0xff, 0xfe];
        bytes.extend_from_slice(&(packed.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(raw.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&packed);
        bytes
    }

    #[test]
    fn compressed_ksd_and_bmp_payloads_share_execution_classification() {
        let state = compressed(
            r#"(const) %["core" => (const) %["currentStorage" => "chapter03.ks", "historyData" => (const) %["data" => (const) ["a native record"]]], "history" => (const) []]"#,
        );
        let mut image = vec![0; 54];
        image[..2].copy_from_slice(b"BM");
        image[2..6].copy_from_slice(&54u32.to_le_bytes());
        image.extend_from_slice(&state);
        assert_eq!(
            (classify_bytes(&state), classify_bytes(&image)),
            (SaveKind::Progress, SaveKind::Progress)
        );
    }

    #[test]
    fn native_options_and_read_flags_are_controls_regardless_of_the_filename() {
        for text in [
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1024]"#,
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1024, "volume" => 0x1.4000000000000p3]"#,
            r#"(const) %["fullScreen" => 1, "windowWidth" => 1024, "audio" => (const) %["volume" => 80]]"#,
            r#"(const) %["trail_chapter_one" => 1, "cg_example" => 1, "language" => "ja"]"#,
        ] {
            assert_eq!(classify_bytes(&compressed(text)), SaveKind::Control);
        }
    }

    #[test]
    fn unfamiliar_state_and_incomplete_or_trailing_records_remain_unknown() {
        let mut broken = compressed(r#"(const) %["fullScreen" => 1, "windowWidth" => 1024]"#);
        broken.pop();
        assert_eq!(classify_bytes(&broken), SaveKind::Unknown);
        let mut trailing = compressed(r#"(const) %["fullScreen" => 1, "windowWidth" => 1024]"#);
        trailing.extend_from_slice(b"extra");
        assert_eq!(classify_bytes(&trailing), SaveKind::Unknown);
        assert_eq!(
            classify_bytes(br#"(const) %["chapter" => "one", "line" => 7]"#),
            SaveKind::Unknown
        );
        assert_eq!(
            classify_bytes(br#"(const) %["currentStorage" => "scene.ks"]"#),
            SaveKind::Unknown
        );
        assert_eq!(classify_bytes(br#"(const) %["currentStorage" => "menu.ks", "mainConductor" => (const) %["pos" => 7]]"#), SaveKind::Unknown);
        assert_eq!(classify_bytes(br#"(const) %["fullScreen" => 1, "windowWidth" => 1280, "scene" => "a story scene"]"#), SaveKind::Unknown);
        assert_eq!(classify_bytes(br#"(const) %["storageName" => "chapter.ks", "curLabel" => "*checkpoint", "curLine" => 12, "fullScreen" => 1, "windowWidth" => 1280]"#), SaveKind::Unknown);
        assert_eq!(classify_bytes(br#"(const) %["opaqueState" => (const) %["resumeTarget" => "*checkpoint"], "fullScreen" => 1, "windowWidth" => 1280]"#), SaveKind::Unknown);
    }

    #[test]
    fn serialized_marker_strings_do_not_masquerade_as_executable_state() {
        let text =
            r#"(const) %["note" => "\"currentStorage\" => \"chapter.ks\"", "fullScreen" => 1]"#;
        assert_eq!(classify_bytes(text.as_bytes()), SaveKind::Unknown);
    }

    #[test]
    fn supported_bit_permutation_and_escaped_unicode_preserve_native_structure() {
        let text = r#"(const) %["currentStorage" => "\x7ae0.ks", "historyData" => (const) %["data" => (const) ["line"]]]"#;
        let mut encoded = vec![0xfe, 0xfe, 1, 0xff, 0xfe];
        encoded.extend(
            text.encode_utf16()
                .flat_map(u16::to_le_bytes)
                .map(|byte| ((byte & 0xaa) >> 1) | ((byte & 0x55) << 1)),
        );
        assert_eq!(classify_bytes(&encoded), SaveKind::Progress);
    }
}
