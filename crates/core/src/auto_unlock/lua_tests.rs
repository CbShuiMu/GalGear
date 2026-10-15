use super::*;
fn function(
    code: Vec<u32>,
    constants: Vec<Constant>,
    children: Vec<Function>,
    params: u8,
    stack: u8,
) -> Function {
    Function {
        name: String::new(),
        code,
        constants,
        children,
        upvalues: vec![(0, 0)],
        params,
        stack,
        code_count: 0,
        code_start: 0,
        constant_count: 0,
        constant_end: 0,
        lines: 0,
        locals: 0,
    }
}
fn getter() -> Function {
    function(
        vec![
            abc(11, 3, 0, 0),
            abc(10, 3, 256, 257),
            abc(10, 3, 258, 259),
            abc(6, 1, 0, 260),
            abc(7, 1, 1, 261),
            abc(7, 2, 3, 256),
            abc(36, 1, 2, 2),
            abc(38, 1, 2, 0),
        ],
        vec![
            Constant::Text("CG_A".into()),
            Constant::Int(7),
            Constant::Text("unused".into()),
            Constant::Int(999),
            Constant::Text("cfunc".into()),
            Constant::Text("LegacyGame__lua_GetFlag".into()),
        ],
        Vec::new(),
        0,
        4,
    )
}
fn write_function(out: &mut Vec<u8>, f: &Function) {
    out.push(0);
    out.extend(0u32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend([f.params, 0, f.stack]);
    out.extend((f.code.len() as u32).to_le_bytes());
    for w in &f.code {
        out.extend(w.to_le_bytes());
    }
    out.extend((f.constants.len() as u32).to_le_bytes());
    for k in &f.constants {
        out.extend(encode_constant(k).unwrap());
    }
    out.extend((f.upvalues.len() as u32).to_le_bytes());
    for &(a, b) in &f.upvalues {
        out.extend([a, b]);
    }
    out.extend((f.children.len() as u32).to_le_bytes());
    for child in &f.children {
        write_function(out, child);
    }
    out.extend([0u8; 12]);
}
fn fixture(size_t: u8) -> Vec<u8> {
    let constructor = function(
        vec![44 | (1 << 6), abc(10, 0, 256, 1), abc(38, 0, 1, 0)],
        vec![Constant::Text("GetCgFlag".into())],
        vec![getter()],
        1,
        4,
    );
    let root = function(
        vec![44, abc(8, 0, 256, 0), abc(38, 0, 1, 0)],
        vec![Constant::Text("GalleryCgMenu".into())],
        vec![constructor],
        0,
        2,
    );
    let mut out = b"\x1bLua\x53\0\x19\x93\r\n\x1a\n".to_vec();
    out.extend([4, size_t, 4, 8, 8]);
    out.extend(0x5678u64.to_le_bytes());
    out.extend(370.5f64.to_le_bytes());
    out.push(1);
    write_function(&mut out, &root);
    out
}
#[test]
fn flag_query_ignores_unrelated_numeric_constants() {
    let records = flag_records(&getter()).unwrap();
    assert_eq!(records, BTreeMap::from([("CG_A".into(), 7)]));
}
#[test]
fn dynamic_flag_expressions_are_rejected() {
    let mut f = getter();
    f.code[5] = abc(0, 2, 0, 0);
    assert!(flag_records(&f).is_err());
}
#[test]
fn malformed_register_operands_are_rejected() {
    let mut f = getter();
    f.code[0] = abc(0, 3, 511, 0);
    assert!(validate(&f).is_err());
}
#[test]
fn constructor_patch_preserves_original_code_and_child_records() {
    for size in [4, 8] {
        let data = fixture(size);
        let chunk = parse(&data).unwrap();
        let ctor = &chunk.root.children[0];
        let before = ctor.code.clone();
        let records = flag_records(&ctor.children[0]).unwrap();
        let patched = inject(&data, &[ctor], &BTreeSet::from([7, 55])).unwrap();
        let after = parse(&patched).unwrap();
        let changed = &after.root.children[0];
        assert_eq!(&changed.code[10..], before);
        assert_eq!(flag_records(&changed.children[0]).unwrap(), records);
        assert!(changed.constants.contains(&Constant::Int(55)));
        assert_eq!(
            changed.code[..10].iter().filter(|w| *w & 63 == 36).count(),
            2
        );
    }
}
#[test]
fn truncated_chunks_fail_at_every_boundary() {
    let data = fixture(8);
    for length in 0..data.len() {
        assert!(parse(&data[..length]).is_err(), "length {length}");
    }
}
