use super::*;
fn page() -> Vec<u8> {
    let mut call = vec![0x1c];
    call.extend(b"openCgBrowser\0image.png\0\0\0\0");
    let mut out = vec![1, 2];
    out.extend(17u16.to_le_bytes());
    out.extend(1.0f32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend((16 + call.len() as u32).to_le_bytes());
    out.extend(call);
    out.extend([0xff, 0, 0, 0, 0, 0, 0, 0, 0]);
    out
}
#[test]
fn cg_flags_require_a_browser_call_and_valid_instruction_targets() {
    let data = page();
    let records = differences(&data).unwrap();
    assert_eq!(records[0].flag, 17);
    assert_eq!(records[0].resource, "image.png");
    let mut bad = data.clone();
    bad[12..16].copy_from_slice(&3u32.to_le_bytes());
    assert!(differences(&bad).is_err());
    let mut bad = data;
    bad[17] = b'X';
    assert!(differences(&bad).is_err());
}
#[test]
fn incomplete_cg_scripts_are_rejected() {
    let data = page();
    for length in 0..data.len() - 8 {
        assert!(differences(&data[..length]).is_err());
    }
}
