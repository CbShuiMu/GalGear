use super::{AutoResult, Cursor, failure};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "ws2_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
enum Instruction {
    Condition {
        kind: u8,
        id: u16,
        value: f32,
        yes: u32,
        no: u32,
    },
    Jump(u32),
    Call(String, String),
    SetString,
    Return,
    End,
}
#[derive(Clone, Debug)]
struct Located {
    start: usize,
    end: usize,
    instruction: Instruction,
}
fn string(cursor: &mut Cursor<'_>, wide: bool) -> AutoResult<String> {
    if wide {
        let mut words = Vec::new();
        loop {
            let w = cursor.u16()?;
            if w == 0 {
                break;
            }
            words.push(w);
            if words.len() > 8192 {
                return Err(failure("WS2 string limit"));
            }
        }
        String::from_utf16(&words).map_err(|_| failure("WS2 UTF-16"))
    } else {
        let mut bytes = Vec::new();
        loop {
            let b = cursor.byte()?;
            if b == 0 {
                break;
            }
            bytes.push(b);
            if bytes.len() > 8192 {
                return Err(failure("WS2 string limit"));
            }
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
fn decode(data: &[u8], wide: bool) -> AutoResult<Vec<Located>> {
    let mut cursor = Cursor::new(data);
    let mut result = Vec::new();
    let mut ended = false;
    while cursor.position < data.len() {
        let start = cursor.position;
        let instruction = match cursor.byte()? {
            0 => continue,
            5 => Instruction::Return,
            0xff => {
                if cursor
                    .take(data.len() - cursor.position)?
                    .iter()
                    .any(|v| *v != 0)
                {
                    return Err(failure("WS2 EOF padding"));
                }
                ended = true;
                Instruction::End
            }
            2 | 6 => Instruction::Jump(cursor.u32()?),
            1 => {
                let kind = cursor.byte()?;
                if !matches!(kind, 2 | 128 | 129 | 130 | 192) {
                    return Err(failure("WS2 condition dialect"));
                }
                Instruction::Condition {
                    kind,
                    id: cursor.u16()?,
                    value: f32::from_le_bytes(cursor.take(4)?.try_into().unwrap()),
                    yes: cursor.u32()?,
                    no: cursor.u32()?,
                }
            }
            0x6e => {
                string(&mut cursor, wide)?;
                string(&mut cursor, wide)?;
                Instruction::SetString
            }
            0x1c => {
                let name = string(&mut cursor, wide)?;
                let argument = string(&mut cursor, wide)?;
                cursor.take(3)?;
                Instruction::Call(name, argument)
            }
            _ => {
                return Err(failure(format!(
                    "WS2 unsupported instruction at {start:#x}"
                )));
            }
        };
        result.push(Located {
            start,
            end: cursor.position,
            instruction,
        });
        if ended {
            break;
        }
    }
    if !ended {
        return Err(failure("WS2 missing EOF"));
    }
    let boundaries: BTreeSet<_> = result.iter().map(|v| v.start as u32).collect();
    for item in &result {
        let targets = match item.instruction {
            Instruction::Condition { yes, no, .. } => vec![yes, no],
            Instruction::Jump(t) => vec![t],
            _ => Vec::new(),
        };
        if targets.iter().any(|v| *v != 0 && !boundaries.contains(v)) {
            return Err(failure("WS2 branch target is not an instruction boundary"));
        }
    }
    Ok(result)
}

#[derive(Clone, Debug)]
pub struct Difference {
    pub flag: u16,
    pub resource: String,
    pub offset: usize,
}
/// A flag is accepted only when its seen-state condition guards a decoded
/// CG/movie browser call. Byte-pattern matches inside strings are not records.
pub fn differences(data: &[u8]) -> AutoResult<Vec<Difference>> {
    let program = decode(data, false).or_else(|_| decode(data, true))?;
    let mut records = Vec::new();
    for (i, item) in program.iter().enumerate() {
        if let Instruction::Condition {
            kind: 2,
            id,
            value,
            yes,
            no,
        } = item.instruction
        {
            if value != 1.0 || yes != 0 || no <= item.end as u32 {
                return Err(failure("CG condition is not a positive seen flag"));
            }
            let next = program
                .get(i + 1)
                .ok_or_else(|| failure("CG browser call missing"))?;
            if let Instruction::Call(ref name, ref argument) = next.instruction {
                if !matches!(name.as_str(), "openCgBrowser" | "openMoveBrowser")
                    || next.end > no as usize
                    || argument.is_empty()
                {
                    return Err(failure("Unproven CG browser condition"));
                }
                records.push(Difference {
                    flag: id,
                    resource: argument.clone(),
                    offset: item.start,
                });
            } else {
                return Err(failure("CG flag does not directly guard a browser call"));
            }
        }
    }
    if records.is_empty() {
        return Err(failure("No proven CG variant records"));
    }
    Ok(records)
}

/// The replay router's first condition provides the route section entry.
/// Graphics restoration in the preceding section is not modified.
pub fn replay_routes(data: &[u8]) -> AutoResult<BTreeMap<u32, String>> {
    replay_dialect(data, false).or_else(|_| replay_dialect(data, true))
}
fn replay_dialect(data: &[u8], wide: bool) -> AutoResult<BTreeMap<u32, String>> {
    let mut cursor = Cursor::new(data);
    if cursor.byte()? != 1 || cursor.byte()? != 128 {
        return Err(failure("Replay router header"));
    }
    let selector = cursor.u16()?;
    let value = f32::from_le_bytes(cursor.take(4)?.try_into().unwrap());
    let yes = cursor.u32()?;
    let no = cursor.u32()? as usize;
    if value != 1.0 || yes != 0 || no < 16 || no >= data.len() {
        return Err(failure("Replay route section"));
    }
    cursor.position = no;
    let mut routes = BTreeMap::new();
    let mut ended = false;
    while cursor.position < data.len() {
        let start = cursor.position;
        match cursor.byte()? {
            1 => {
                if cursor.byte()? != 130 || cursor.u16()? != selector {
                    return Err(failure("Replay selector mismatch"));
                }
                let value = f32::from_le_bytes(cursor.take(4)?.try_into().unwrap());
                let yes = cursor.u32()?;
                let no = cursor.u32()? as usize;
                if yes != 0
                    || !value.is_finite()
                    || value.fract() != 0.0
                    || !(1.0..=10000.0).contains(&value)
                    || no <= cursor.position
                    || no >= data.len()
                    || cursor.byte()? != 7
                {
                    return Err(failure("Replay route branch"));
                }
                let name = string(&mut cursor, wide)?;
                if cursor.position != no
                    || name.is_empty()
                    || routes.insert(value as u32, name).is_some()
                {
                    return Err(failure("Replay route target"));
                }
            }
            5 | 0 => {}
            6 => {
                let target = cursor.u32()? as usize;
                if target != cursor.position {
                    return Err(failure("Replay footer jump"));
                }
            }
            7 => {
                if !string(&mut cursor, wide)?.eq_ignore_ascii_case("title")
                    || data.get(cursor.position) != Some(&0xff)
                {
                    return Err(failure("Replay fallback title"));
                }
            }
            0xff => {
                if cursor
                    .take(data.len() - cursor.position)?
                    .iter()
                    .any(|v| *v != 0)
                {
                    return Err(failure("Replay EOF"));
                }
                ended = true;
                break;
            }
            _ => {
                return Err(failure(format!(
                    "Replay unsupported instruction at {start:#x}"
                )));
            }
        }
    }
    if !ended || routes.is_empty() {
        return Err(failure("Replay routes incomplete"));
    }
    Ok(routes)
}
