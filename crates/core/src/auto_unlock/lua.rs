use super::{AutoResult, Cursor, failure};
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "lua_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Nil,
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
}
#[derive(Clone, Debug)]
pub struct Function {
    pub name: String,
    pub code: Vec<u32>,
    pub constants: Vec<Constant>,
    pub children: Vec<Function>,
    pub upvalues: Vec<(u8, u8)>,
    pub params: u8,
    pub stack: u8,
    code_count: usize,
    code_start: usize,
    constant_count: usize,
    constant_end: usize,
    lines: usize,
    locals: usize,
}

pub struct Chunk {
    pub root: Function,
    pub size_t: usize,
}
struct Reader<'a> {
    cursor: Cursor<'a>,
    size_t: usize,
    nodes: usize,
}
impl Reader<'_> {
    fn count(&mut self, limit: usize) -> AutoResult<usize> {
        let n = self.cursor.u32()? as usize;
        if n > limit {
            Err(failure("Lua vector limit"))
        } else {
            Ok(n)
        }
    }
    fn string(&mut self) -> AutoResult<String> {
        let mut size = self.cursor.byte()? as usize;
        if size == 255 {
            size = usize::try_from(self.cursor.integer(self.size_t)?)
                .map_err(|_| failure("Lua string size"))?;
        }
        if size == 0 {
            return Ok(String::new());
        }
        if size > 4_000_000 {
            return Err(failure("Lua string size"));
        }
        String::from_utf8(self.cursor.take(size - 1)?.to_vec())
            .map_err(|_| failure("Lua UTF-8 string"))
    }
    fn function(&mut self, depth: usize) -> AutoResult<Function> {
        self.nodes += 1;
        if depth > 64 || self.nodes > 10_000 {
            return Err(failure("Lua prototype limit"));
        }
        self.string()?;
        self.cursor.u32()?;
        self.cursor.u32()?;
        let params = self.cursor.byte()?;
        self.cursor.byte()?;
        let stack = self.cursor.byte()?;
        if stack < params || stack == 0 {
            return Err(failure("Lua stack"));
        }
        let code_count = self.cursor.position;
        let count = self.count(1_000_000)?;
        let code_start = self.cursor.position;
        let mut code = Vec::with_capacity(count);
        for _ in 0..count {
            let word = self.cursor.u32()?;
            if word & 63 > 46 {
                return Err(failure("Lua opcode"));
            }
            code.push(word);
        }
        let constant_count = self.cursor.position;
        let count = self.count(500_000)?;
        let mut constants = Vec::with_capacity(count);
        for _ in 0..count {
            constants.push(match self.cursor.byte()? {
                0 => Constant::Nil,
                1 => {
                    let b = self.cursor.byte()?;
                    if b > 1 {
                        return Err(failure("Lua boolean"));
                    }
                    Constant::Bool(b != 0)
                }
                3 => Constant::Number(f64::from_le_bytes(self.cursor.take(8)?.try_into().unwrap())),
                19 => Constant::Int(self.cursor.integer(8)? as i64),
                4 | 20 => Constant::Text(self.string()?),
                _ => return Err(failure("Lua constant tag")),
            });
        }
        let constant_end = self.cursor.position;
        let count = self.count(255)?;
        let mut upvalues = Vec::new();
        for _ in 0..count {
            upvalues.push((self.cursor.byte()?, self.cursor.byte()?));
        }
        let count = self.count(10_000)?;
        let mut children = Vec::new();
        for _ in 0..count {
            children.push(self.function(depth + 1)?);
        }
        let lines = self.count(1_000_000)?;
        self.cursor.take(lines * 4)?;
        if lines != 0 && lines != code.len() {
            return Err(failure("Lua debug line count"));
        }
        let count = self.count(100_000)?;
        let locals = count;
        for _ in 0..count {
            self.string()?;
            let start = self.cursor.u32()? as usize;
            let end = self.cursor.u32()? as usize;
            if start > end || end > code.len() {
                return Err(failure("Lua local range"));
            }
        }
        let count = self.count(255)?;
        for _ in 0..count {
            self.string()?;
        }
        for pair in code.windows(2) {
            let w = pair[0];
            let next = pair[1];
            let a = ((w >> 6) & 255) as usize;
            let b = (next >> 23) as usize;
            let c = ((next >> 14) & 511) as usize;
            if w & 63 == 44 && matches!(next & 63, 8 | 10) && c == a && b & 256 != 0 {
                let name = match constants.get(b & 255) {
                    Some(Constant::Text(s)) => s.clone(),
                    _ => return Err(failure("Lua closure name")),
                };
                let child = children
                    .get_mut((w >> 14) as usize)
                    .ok_or_else(|| failure("Lua closure index"))?;
                child.name = name;
            }
        }
        Ok(Function {
            name: String::new(),
            code,
            constants,
            children,
            upvalues,
            params,
            stack,
            code_count,
            code_start,
            constant_count,
            constant_end,
            lines,
            locals,
        })
    }
}

pub fn parse(data: &[u8]) -> AutoResult<Chunk> {
    let mut cursor = Cursor::new(data);
    if cursor.take(12)? != b"\x1bLua\x53\0\x19\x93\r\n\x1a\n" {
        return Err(failure("Lua 5.3 header"));
    }
    let sizes = cursor.take(5)?;
    if sizes[0] != 4 || !matches!(sizes[1], 4 | 8) || sizes[2..] != [4, 8, 8] {
        return Err(failure("Lua native sizes"));
    }
    let size_t = sizes[1] as usize;
    if cursor.integer(8)? != 0x5678
        || f64::from_le_bytes(cursor.take(8)?.try_into().unwrap()) != 370.5
        || cursor.byte()? != 1
    {
        return Err(failure("Lua byte order or environment"));
    }
    let mut reader = Reader {
        cursor,
        size_t,
        nodes: 0,
    };
    let root = reader.function(0)?;
    if reader.cursor.position != data.len() {
        return Err(failure("Lua trailing data"));
    }
    let mut functions = Vec::new();
    root.all(&mut functions);
    for function in functions {
        validate(function)?;
    }
    Ok(Chunk { root, size_t })
}

fn validate(f: &Function) -> AutoResult<()> {
    if f.code.is_empty() {
        return Err(failure("Empty Lua code"));
    }
    let reg = |r: usize| -> AutoResult<()> {
        if r < f.stack as usize {
            Ok(())
        } else {
            Err(failure("Lua register bounds"))
        }
    };
    let rk = |r: usize| -> AutoResult<()> {
        if r & 256 != 0 {
            if r & 255 < f.constants.len() {
                Ok(())
            } else {
                Err(failure("Lua RK bounds"))
            }
        } else {
            reg(r)
        }
    };
    for (pc, &w) in f.code.iter().enumerate() {
        let op = w & 63;
        let a = ((w >> 6) & 255) as usize;
        let b = (w >> 23) as usize;
        let c = ((w >> 14) & 511) as usize;
        if !matches!(op, 8 | 30..=33 | 46) {
            reg(a)?;
        }
        match op {
            0 | 25..=28 => reg(b)?,
            1 => {
                if (w >> 14) as usize >= f.constants.len() {
                    return Err(failure("Lua LOADK bounds"));
                }
            }
            4 => reg(a + b)?,
            6 => {
                if b >= f.upvalues.len() {
                    return Err(failure("Lua upvalue bounds"));
                }
                rk(c)?;
            }
            7 | 12 => {
                reg(b)?;
                rk(c)?;
            }
            8 => {
                if a >= f.upvalues.len() {
                    return Err(failure("Lua upvalue bounds"));
                }
                rk(b)?;
                rk(c)?;
            }
            10 | 13..=24 | 31..=33 => {
                rk(b)?;
                rk(c)?;
            }
            29 => {
                reg(b)?;
                reg(c)?;
            }
            35 => reg(b)?,
            36 | 37 => {
                if b > 0 {
                    reg(a + b - 1)?;
                }
            }
            38 => {
                if b > 1 {
                    reg(a + b - 2)?;
                }
            }
            43 => {
                if b > 0 {
                    reg(a + b)?;
                }
            }
            44 => {
                if (w >> 14) as usize >= f.children.len() {
                    return Err(failure("Lua prototype bounds"));
                }
            }
            _ => {}
        }
        if matches!(op, 30 | 39 | 40 | 42) {
            let target = pc as i64 + 1 + (w >> 14) as i64 - 131071;
            if target < 0 || target >= f.code.len() as i64 {
                return Err(failure("Lua branch bounds"));
            }
        }
    }
    Ok(())
}

impl Function {
    pub fn all<'a>(&'a self, output: &mut Vec<&'a Function>) {
        output.push(self);
        for child in &self.children {
            child.all(output);
        }
    }
}
#[derive(Clone, Debug, Default)]
enum Value {
    #[default]
    Unknown,
    Int(u16),
    Text(String),
    Table(usize),
    Flags(BTreeSet<u16>),
    NativeGetFlag,
}
fn domain(value: &Value) -> BTreeSet<u16> {
    match value {
        Value::Int(v) => BTreeSet::from([*v]),
        Value::Flags(v) => v.clone(),
        _ => BTreeSet::new(),
    }
}
fn constant(k: &Constant) -> Value {
    match k {
        Constant::Int(v) if (0..=u16::MAX as i64).contains(v) => Value::Int(*v as u16),
        Constant::Text(v) => Value::Text(v.clone()),
        _ => Value::Unknown,
    }
}

/// Follow table values to the native GetFlag argument, rather than selecting
/// every integer constant or assuming a game-specific numeric range.
pub fn flag_records(function: &Function) -> AutoResult<BTreeMap<String, u16>> {
    let mut registers = vec![Value::Unknown; 256];
    let mut tables: BTreeMap<usize, BTreeMap<String, Value>> = BTreeMap::new();
    let mut literals = BTreeMap::new();
    let mut queried = BTreeSet::new();
    let rk = |v: usize, regs: &[Value]| -> AutoResult<Value> {
        if v & 256 != 0 {
            function
                .constants
                .get(v & 255)
                .map(constant)
                .ok_or_else(|| failure("Lua RK bounds"))
        } else {
            Ok(regs[v].clone())
        }
    };
    for (pc, &word) in function.code.iter().enumerate() {
        let op = word & 63;
        let a = ((word >> 6) & 255) as usize;
        let b = (word >> 23) as usize;
        let c = ((word >> 14) & 511) as usize;
        match op {
            0 => registers[a] = registers[b].clone(),
            1 => {
                registers[a] = function
                    .constants
                    .get((word >> 14) as usize)
                    .map(constant)
                    .ok_or_else(|| failure("Lua LOADK bounds"))?
            }
            3 => registers[a] = Value::Unknown,
            // Nil in a conditional unavailable-item branch must not erase the
            // finite domain established by the gallery's table lookup.
            4 => {
                for register in registers.iter_mut().skip(a).take(b + 1) {
                    if !matches!(register, Value::Flags(_)) {
                        *register = Value::Unknown;
                    }
                }
            }
            6 => registers[a] = rk(c, &registers)?,
            7 => {
                let table = registers[b].clone();
                let key = rk(c, &registers)?;
                registers[a] = match (table, key) {
                    (Value::Text(t), Value::Text(k))
                        if t == "cfunc" && k == "LegacyGame__lua_GetFlag" =>
                    {
                        Value::NativeGetFlag
                    }
                    (Value::Table(t), Value::Text(k)) => tables
                        .get(&t)
                        .and_then(|v| v.get(&k))
                        .cloned()
                        .unwrap_or_default(),
                    (Value::Table(t), _) => Value::Flags(
                        tables
                            .get(&t)
                            .map(|v| v.values().flat_map(domain).collect())
                            .unwrap_or_default(),
                    ),
                    _ => Value::Unknown,
                };
            }
            10 => {
                if let Value::Table(t) = registers[a] {
                    let key = rk(b, &registers)?;
                    let value = rk(c, &registers)?;
                    if let Value::Text(key) = key {
                        if let Value::Int(id) = value {
                            literals.insert(key.clone(), id);
                        }
                        tables.entry(t).or_default().insert(key, value);
                    }
                }
            }
            11 => {
                tables.insert(pc, BTreeMap::new());
                registers[a] = Value::Table(pc);
            }
            36 | 37 => {
                if matches!(registers[a], Value::NativeGetFlag) {
                    if b != 2 || a == 255 {
                        return Err(failure("GetFlag signature"));
                    }
                    let ids = domain(&registers[a + 1]);
                    if ids.is_empty() {
                        return Err(failure("Dynamic gallery flag expression"));
                    }
                    queried.extend(ids);
                }
                registers[a] = Value::Unknown;
            }
            43 => {
                if let Value::Table(t) = registers[a] {
                    if b == 0 {
                        return Err(failure("Dynamic Lua gallery array"));
                    }
                    for (i, value) in registers.iter().skip(a + 1).take(b).enumerate() {
                        tables
                            .entry(t)
                            .or_default()
                            .insert(format!("#{i}"), value.clone());
                    }
                }
            }
            30..=35 | 38..=42 | 44..=46 | 8 | 9 => {}
            // Arithmetic may turn an ID into a different record; it is not
            // inferred as a flag unless a later explicit table lookup proves it.
            _ => registers[a] = Value::Unknown,
        }
    }
    let records: BTreeMap<_, _> = literals
        .into_iter()
        .filter(|(_, id)| queried.contains(id))
        .collect();
    let proven: BTreeSet<_> = records.values().copied().collect();
    if queried.is_empty() || queried != proven {
        return Err(failure("Gallery records are not a finite named flag table"));
    }
    Ok(records)
}

fn abc(op: u32, a: u32, b: u32, c: u32) -> u32 {
    op | (a << 6) | (c << 14) | (b << 23)
}

pub fn setter_is_used(root: &Function) -> bool {
    let mut functions = Vec::new();
    root.all(&mut functions);
    functions.into_iter().any(|f| {
        f.code.windows(5).any(|w| {
            let a = (w[0] >> 6) & 255;
            let text = |word: u32, name: &str| {
                ((word >> 14) & 511) & 256 != 0
                    && f.constants.get(((word >> 14) & 255) as usize)
                        == Some(&Constant::Text(name.into()))
            };
            w[0] & 63 == 6
                && text(w[0], "cfunc")
                && w[1] & 63 == 7
                && ((w[1] >> 6) & 255) == a
                && (w[1] >> 23) == a
                && text(w[1], "LegacyGame__lua_SetFlag")
                && w[2] & 63 == 1
                && ((w[2] >> 6) & 255) == a + 1
                && w[3] & 63 == 3
                && ((w[3] >> 6) & 255) == a + 2
                && ((w[3] >> 23) & 511) <= 1
                && w[4] == abc(36, a, 3, 1)
        })
    })
}

pub fn literal_returns(function: &Function) -> AutoResult<Vec<Constant>> {
    let mut registers: Vec<Option<Constant>> = vec![None; 256];
    for &w in &function.code {
        let op = w & 63;
        let a = ((w >> 6) & 255) as usize;
        match op {
            1 => {
                registers[a] = Some(
                    function
                        .constants
                        .get((w >> 14) as usize)
                        .ok_or_else(|| failure("Literal return constant"))?
                        .clone(),
                )
            }
            3 => registers[a] = Some(Constant::Bool((w >> 23) & 511 != 0)),
            38 => {
                let b = (w >> 23) as usize;
                if b > 1 && a + b - 1 <= registers.len() {
                    return registers[a..a + b - 1]
                        .iter()
                        .cloned()
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| failure("Unproven literal return"));
                }
            }
            _ => return Err(failure("Dynamic metadata return")),
        }
    }
    Err(failure("Missing literal return"))
}
fn encode_constant(k: &Constant) -> AutoResult<Vec<u8>> {
    match k {
        Constant::Int(v) => {
            let mut vout = vec![19];
            vout.extend(v.to_le_bytes());
            Ok(vout)
        }
        Constant::Text(s) if s.len() < 254 => {
            let mut out = vec![4, (s.len() + 1) as u8];
            out.extend(s.as_bytes());
            Ok(out)
        }
        _ => Err(failure("Patch constant encoding")),
    }
}
struct Edit {
    start: usize,
    end: usize,
    data: Vec<u8>,
}
/// Prefix relative-PC Lua code. Original branch distances and constant indexes
/// remain unchanged; no WS2 startup addresses are patched.
pub fn inject(data: &[u8], functions: &[&Function], ids: &BTreeSet<u16>) -> AutoResult<Vec<u8>> {
    if ids.is_empty() || ids.len() > 10_000 {
        return Err(failure("Unlock flag count"));
    }
    let mut edits = Vec::new();
    for f in functions {
        if f.params != 1
            || f.stack < 4
            || f.upvalues != [(0, 0)]
            || !f
                .children
                .iter()
                .any(|v| matches!(v.name.as_str(), "GetCgFlag" | "GetSceneFlag"))
        {
            return Err(failure("Gallery constructor environment"));
        }
        if f.lines != 0 || f.locals != 0 {
            return Err(failure("Unstripped gallery constructor debug locals"));
        }
        let mut constants = f.constants.clone();
        let mut appended = Vec::new();
        let mut add = |k: Constant| -> AutoResult<u32> {
            if let Some(i) = constants.iter().position(|v| *v == k) {
                return Ok(i as u32);
            }
            let i = constants.len() as u32;
            appended.extend(encode_constant(&k)?);
            constants.push(k);
            Ok(i)
        };
        let cfunc = add(Constant::Text("cfunc".into()))?;
        let setter = add(Constant::Text("LegacyGame__lua_SetFlag".into()))?;
        if cfunc > 255 || setter > 255 {
            return Err(failure("Gallery RK constant capacity"));
        }
        let mut prefix = Vec::new();
        for &id in ids {
            let index = add(Constant::Int(id as i64))?;
            if index >= 262144 {
                return Err(failure("Lua constant capacity"));
            }
            for word in [
                abc(6, 1, 0, 256 + cfunc),
                abc(7, 1, 1, 256 + setter),
                1 | (2 << 6) | (index << 14),
                abc(3, 3, 1, 0),
                abc(36, 1, 3, 1),
            ] {
                prefix.extend(word.to_le_bytes());
            }
        }
        add(Constant::Text("GalGear.auto_unlock.v1".into()))?;
        edits.push(Edit {
            start: f.code_count,
            end: f.code_count + 4,
            data: ((f.code.len() + prefix.len() / 4) as u32)
                .to_le_bytes()
                .to_vec(),
        });
        edits.push(Edit {
            start: f.code_start,
            end: f.code_start,
            data: prefix,
        });
        edits.push(Edit {
            start: f.constant_count,
            end: f.constant_count + 4,
            data: (constants.len() as u32).to_le_bytes().to_vec(),
        });
        edits.push(Edit {
            start: f.constant_end,
            end: f.constant_end,
            data: appended,
        });
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut output = data.to_vec();
    for e in edits {
        output.splice(e.start..e.end, e.data);
    }
    parse(&output)?;
    Ok(output)
}
