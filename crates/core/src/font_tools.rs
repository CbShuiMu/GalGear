//! 游戏字体参数、架构识别和版本化通信协议；不实现字体或引擎 Hook。
use galgear_i18n::Message;
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};

pub(crate) fn error(detail: impl ToString) -> Message {
    Message::detail("font.error", detail)
}

/// 目标 PE 的架构；不根据操作系统或启动器的位数推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X86,
    X64,
}
impl Architecture {
    /// 随包组件的目录名。
    pub fn directory(self) -> &'static str {
        match self {
            Self::X86 => "x86",
            Self::X64 => "x64",
        }
    }
}

/// 有界读取 DOS、PE 和可选头；ARM、DLL、截断或不一致的头均拒绝。
pub fn executable_architecture(reader: &mut (impl Read + Seek)) -> Result<Architecture, Message> {
    let mut dos = [0; 64];
    reader.read_exact(&mut dos).map_err(error)?;
    if &dos[..2] != b"MZ" {
        return Err(error("Invalid DOS header"));
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().map_err(error)?);
    if !(64..=16 * 1024 * 1024).contains(&offset) {
        return Err(error("Invalid PE offset"));
    }
    reader.seek(SeekFrom::Start(offset.into())).map_err(error)?;
    let mut header = [0; 26];
    reader.read_exact(&mut header).map_err(error)?;
    if &header[..4] != b"PE\0\0"
        || u16::from_le_bytes([header[20], header[21]]) < 2
        || u16::from_le_bytes([header[22], header[23]]) & 0x2000 != 0
    {
        return Err(error("Invalid executable PE header"));
    }
    match (
        u16::from_le_bytes([header[4], header[5]]),
        u16::from_le_bytes([header[24], header[25]]),
    ) {
        (0x14c, 0x10b) => Ok(Architecture::X86),
        (0x8664, 0x20b) => Ok(Architecture::X64),
        _ => Err(error("Unsupported or inconsistent PE architecture")),
    }
}

/// 按游戏保存的可逆字体替换参数；空字体名只调节大小，不替换字体族。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub face: String,
    pub height_percent: u32,
    pub width_percent: u32,
    pub char_spacing: i32,
    pub line_spacing: i32,
    pub weight: u32,
    pub vertical_metrics: Option<VerticalMetrics>,
    pub charset_spoof: Option<CharsetSpoof>,
    pub mapping: Mapping,
    pub mapping_codepage: u32,
}

/// 上升部、下降部均以 em 的千分比表示，下降部必须为负数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerticalMetrics {
    pub ascent: i32,
    pub descent: i32,
}

/// 显式选择伪装源和目标字符集，不从字体名称推断文本编码。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharsetSpoof {
    pub from: u32,
    pub to: u32,
}

/// 上游已有的运行时映射；不在前端复制映射表。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u32)]
pub enum Mapping {
    #[default]
    Off = 0,
    Japanese = 1,
    TraditionalToSimplified = 2,
    SimplifiedToTraditional = 3,
}

/// v2 固定宽度数据包；新后端必须继续接受此布局或拒绝版本协商。
pub const PACKET_SIZE: usize = 128;
impl Default for Profile {
    fn default() -> Self {
        Self {
            face: String::new(),
            height_percent: 100,
            width_percent: 100,
            char_spacing: 0,
            line_spacing: 0,
            weight: 400,
            vertical_metrics: None,
            charset_spoof: None,
            mapping: Mapping::Off,
            mapping_codepage: 932,
        }
    }
}
impl Profile {
    /// 对齐 Win32 LF_FACESIZE，防止截断字体名和异常缩放。
    pub fn validate(&self) -> Result<(), Message> {
        if self.face.encode_utf16().count() > 31
            || self.face.chars().any(char::is_control)
            || !(50..=300).contains(&self.height_percent)
            || !(50..=300).contains(&self.width_percent)
            || !(-20..=60).contains(&self.char_spacing)
            || !(-80..=200).contains(&self.line_spacing)
            || !(100..=900).contains(&self.weight)
            || self.vertical_metrics.is_some_and(|m| {
                !(100..=2000).contains(&m.ascent) || !(-2000..=-1).contains(&m.descent)
            })
            || self.charset_spoof.is_some_and(|s| {
                ![1, 128, 129, 134, 136].contains(&s.from)
                    || ![1, 128, 129, 134, 136].contains(&s.to)
            })
            || ![0, 932, 936, 949, 950, 65001].contains(&self.mapping_codepage)
            || (self.face.is_empty()
                && (self.vertical_metrics.is_some() || self.charset_spoof.is_some()))
        {
            return Err(Message::new("font.invalid"));
        }
        Ok(())
    }
    /// IPC v2 小端编码；不携带地址、路径或平台相关整数。
    pub fn packet(&self) -> Result<[u8; PACKET_SIZE], Message> {
        self.validate()?;
        let mut bytes = [0u8; PACKET_SIZE];
        for (i, value) in [
            2u32,
            PACKET_SIZE as u32,
            self.height_percent,
            self.width_percent,
        ]
        .iter()
        .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, c) in self.face.encode_utf16().enumerate() {
            bytes[16 + i * 2..18 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        let metrics = self.vertical_metrics.unwrap_or(VerticalMetrics {
            ascent: 880,
            descent: -120,
        });
        let spoof = self
            .charset_spoof
            .unwrap_or(CharsetSpoof { from: 134, to: 128 });
        for (i, value) in [
            self.char_spacing as u32,
            self.line_spacing as u32,
            self.weight,
            u32::from(self.vertical_metrics.is_some()),
            metrics.ascent as u32,
            metrics.descent as u32,
            u32::from(self.charset_spoof.is_some()),
            spoof.from,
            spoof.to,
            self.mapping as u32,
            self.mapping_codepage,
            0,
        ]
        .iter()
        .enumerate()
        {
            bytes[80 + i * 4..84 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Architecture, CharsetSpoof, Mapping, Profile, VerticalMetrics, executable_architecture,
    };
    use std::io::Cursor;
    #[test]
    fn pe_architecture_rejects_mismatches_and_truncation() {
        for (machine, magic, expected) in [
            (0x14cu16, 0x10bu16, Architecture::X86),
            (0x8664, 0x20b, Architecture::X64),
        ] {
            let mut data = vec![0; 90];
            data[..2].copy_from_slice(b"MZ");
            data[60] = 64;
            data[64..68].copy_from_slice(b"PE\0\0");
            data[68..70].copy_from_slice(&machine.to_le_bytes());
            data[84] = 2;
            data[88..90].copy_from_slice(&magic.to_le_bytes());
            assert_eq!(
                executable_architecture(&mut Cursor::new(&data)).unwrap(),
                expected
            );
            data[89] = 3;
            assert!(executable_architecture(&mut Cursor::new(&data)).is_err());
            assert!(executable_architecture(&mut Cursor::new(&data[..70])).is_err());
        }
    }
    #[test]
    fn packet_is_fixed_width_and_validates_utf16_and_scaling() {
        let mut p = Profile {
            face: "微软雅黑".into(),
            height_percent: 125,
            width_percent: 90,
            ..Default::default()
        };
        let packet = p.packet().unwrap();
        assert_eq!(packet[8], 125);
        assert_eq!(&packet[78..80], &[0, 0]);
        p.face = "😀".repeat(16);
        assert!(p.packet().is_err());
        p.face.clear();
        p.height_percent = 0;
        assert!(p.validate().is_err());
    }

    #[test]
    fn old_profiles_get_neutral_runtime_defaults() {
        let old: Profile =
            serde_json::from_str(r#"{"face":"Arial","height_percent":125,"width_percent":90}"#)
                .unwrap();
        assert_eq!(old.char_spacing, 0);
        assert_eq!(old.mapping, Mapping::Off);
        assert_eq!(old.vertical_metrics, None);
        assert_eq!(old.charset_spoof, None);
        assert_eq!(old.weight, 400);
        assert_eq!(
            serde_json::from_str::<Profile>(&serde_json::to_string(&old).unwrap()).unwrap(),
            old
        );
    }

    #[test]
    fn runtime_packet_preserves_signed_metrics_and_explicit_mapping() {
        let p = Profile {
            face: "Arial".into(),
            char_spacing: -2,
            line_spacing: -8,
            weight: 700,
            vertical_metrics: Some(VerticalMetrics {
                ascent: 900,
                descent: -200,
            }),
            charset_spoof: Some(CharsetSpoof { from: 134, to: 128 }),
            mapping: Mapping::SimplifiedToTraditional,
            mapping_codepage: 936,
            ..Default::default()
        };
        let bytes = p.packet().unwrap();
        let at = |offset| i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!((at(0), at(4)), (2, 128));
        assert_eq!((at(80), at(84), at(88)), (-2, -8, 700));
        assert_eq!((at(92), at(96), at(100)), (1, 900, -200));
        assert_eq!((at(104), at(108), at(112)), (1, 134, 128));
        assert_eq!((at(116), at(120), at(124)), (3, 936, 0));
    }

    #[test]
    fn rejects_unusable_metrics_and_unknown_encodings() {
        for p in [
            Profile {
                char_spacing: 61,
                ..Default::default()
            },
            Profile {
                line_spacing: -81,
                ..Default::default()
            },
            Profile {
                weight: 901,
                ..Default::default()
            },
            Profile {
                mapping_codepage: 42,
                ..Default::default()
            },
            Profile {
                face: "Arial".into(),
                vertical_metrics: Some(VerticalMetrics {
                    ascent: 880,
                    descent: 0,
                }),
                ..Default::default()
            },
            Profile {
                face: "Arial".into(),
                charset_spoof: Some(CharsetSpoof { from: 999, to: 128 }),
                ..Default::default()
            },
            Profile {
                vertical_metrics: Some(VerticalMetrics {
                    ascent: 880,
                    descent: -120,
                }),
                ..Default::default()
            },
        ] {
            assert!(p.validate().is_err(), "{p:?}");
        }
    }
}
