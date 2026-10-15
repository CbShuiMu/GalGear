use super::*;
fn archive() -> Vec<u8> {
    let names = ["menu.lua", "asset.png"];
    let payloads = [b"script".as_slice(), b"image".as_slice()];
    let mut table = Vec::new();
    let mut payload: Vec<u8> = Vec::new();
    for (name, data) in names.into_iter().zip(payloads) {
        table.extend((data.len() as u32).to_le_bytes());
        table.extend((payload.len() as u32).to_le_bytes());
        for w in name.encode_utf16().chain(std::iter::once(0)) {
            table.extend(w.to_le_bytes());
        }
        payload.extend(data);
    }
    let mut out = 2u32.to_le_bytes().to_vec();
    out.extend((table.len() as u32).to_le_bytes());
    out.extend(table);
    out.extend(payload);
    out
}
#[test]
fn replacement_preserves_unrelated_assets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Script.arc");
    let data = archive();
    fs::write(&path, &data).unwrap();
    let entries = index(&path).unwrap();
    let patched = replace(&data, &entries, "menu.lua", b"longer script").unwrap();
    fs::write(&path, &patched).unwrap();
    let entries = index(&path).unwrap();
    assert_eq!(read(&path, &entries[1]).unwrap(), b"image");
    assert_eq!(read(&path, &entries[0]).unwrap(), b"longer script");
}
#[test]
fn out_of_file_entries_and_case_collisions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Script.arc");
    let mut data = archive();
    data[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    fs::write(&path, data).unwrap();
    assert!(index(&path).is_err());
    let mut data = archive();
    let from = "asset.png"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let position = data.windows(from.len()).position(|v| v == from).unwrap();
    let to = "MENU.LUA\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    data[position..position + to.len()].copy_from_slice(&to);
    fs::write(&path, data).unwrap();
    assert!(index(&path).is_err());
}
