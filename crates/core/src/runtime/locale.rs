//! LE/LEP 配置读取与首次迁移；不启动工具，不修改已存在的 LEP 配置。
use serde::Serialize;
use std::{fs, io::Write, path::Path};

#[derive(Clone, Debug, Serialize)]
/// 启动配置的只读摘要；GUID 用于调用 LE/LEP 的 `-runas` 入口。
pub struct LocaleProfile {
    pub guid: String,
    pub name: String,
    pub locale: String,
    pub admin: bool,
}

fn is_plus(tool: &Path) -> bool {
    tool.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        name.eq_ignore_ascii_case("LEPProc_x86.exe") || name.eq_ignore_ascii_case("LEPProc_x64.exe")
    })
}

/// 只读所选启动器的全局配置，保留外部旧版 LE 的配置文件语义。
pub fn locale_profiles(tool: &Path) -> Result<Vec<LocaleProfile>, String> {
    let name = if is_plus(tool) {
        "LEPConfig.xml"
    } else {
        "LEConfig.xml"
    };
    let path = tool.parent().ok_or("Missing tool directory")?.join(name);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let doc = roxmltree::Document::parse(&text).map_err(|e| e.to_string())?;
    let profiles = doc
        .root_element()
        .children()
        .find(|n| n.has_tag_name("Profiles"))
        .ok_or("Missing locale profiles")?;
    profiles
        .children()
        .filter(|n| n.has_tag_name("Profile"))
        .map(|node| {
            Ok(LocaleProfile {
                guid: node.attribute("Guid").ok_or("Missing profile GUID")?.into(),
                name: node.attribute("Name").unwrap_or_default().into(),
                locale: field(node, "Location").unwrap_or_default().into(),
                admin: field(node, "RunAsAdmin")
                    .unwrap_or("false")
                    .eq_ignore_ascii_case("true"),
            })
        })
        .collect()
}

fn field<'a>(node: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.children()
        .find(|n| n.has_tag_name(name))
        .and_then(|n| n.text())
}

/// 首次使用 LEP 时原子创建配置：优先迁移相邻 LEConfig.xml，否则使用日文默认配置。
/// 保留旧文件及所有 GUID；已有 LEP 配置不覆盖，旧配置损坏时返回错误而不重置。
pub fn prepare_locale_config(tool: &Path) -> Result<(), String> {
    if !is_plus(tool) {
        return Ok(());
    }
    let directory = tool.parent().ok_or("Missing tool directory")?;
    let destination = directory.join("LEPConfig.xml");
    if destination.try_exists().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let legacy = directory.join("LEConfig.xml");
    let text = match fs::read_to_string(&legacy) {
        Ok(text) => migrate(&text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            include_str!("locale-default.xml").into()
        }
        Err(e) => return Err(format!("{}: {e}", legacy.display())),
    };
    let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|e| e.to_string())?;
    temporary
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    match temporary.persist_noclobber(destination) {
        Ok(_) => Ok(()),
        // 另一个工具实例可能刚完成初始化；不覆盖它的配置。
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn migrate(text: &str) -> Result<String, String> {
    let doc = roxmltree::Document::parse(text).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name("LEConfig") {
        return Err("Invalid legacy locale config".into());
    }
    let profiles = root
        .children()
        .find(|n| n.has_tag_name("Profiles"))
        .ok_or("Missing legacy locale profiles")?;
    let mut result =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<LEPConfig><Profiles>\n");
    let mut count = 0;
    for profile in profiles.children().filter(|n| n.has_tag_name("Profile")) {
        for name in ["Guid", "Name", "MainMenu"] {
            if profile.attribute(name).is_none() {
                return Err(format!("Missing legacy profile {name}"));
            }
        }
        for name in ["Parameter", "Location", "Timezone"] {
            if !profile.children().any(|n| n.has_tag_name(name)) {
                return Err(format!("Missing legacy profile {name}"));
            }
        }
        let flag = |name, default| -> Result<bool, String> {
            match field(profile, name) {
                None => Ok(default),
                Some(value) if value.eq_ignore_ascii_case("true") => Ok(true),
                Some(value) if value.eq_ignore_ascii_case("false") => Ok(false),
                Some(_) => Err(format!("Invalid legacy profile {name}")),
            }
        };
        let registry = if !flag("RedirectRegistry", true)? {
            0
        } else if flag("IsAdvancedRedirection", false)? {
            2
        } else {
            1
        };
        // 复制 XML 节点的原始切片，保留转义、参数、未知字段与 GUID；仅增加 LEP 所需字段。
        let original = &text[profile.range()];
        let close = original.rfind("</").ok_or("Invalid legacy profile")?;
        result.push_str(&original[..close]);
        result.push_str(&format!("<RegistryRedirectionMode>{registry}</RegistryRedirectionMode><HookUILanguageMode>1</HookUILanguageMode>"));
        result.push_str(&original[close..]);
        result.push('\n');
        count += 1;
    }
    if count == 0 {
        return Err("Empty legacy locale profiles".into());
    }
    result.push_str("</Profiles></LEPConfig>\n");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{locale_profiles, prepare_locale_config};
    use std::fs;

    const LEGACY: &str = r#"<LEConfig><Profiles><Profile Guid="custom" Name="日本語 &amp; game" MainMenu="false"><Parameter>--name &quot;a b&quot;</Parameter><Location>ja-JP</Location><Timezone>Tokyo Standard Time</Timezone><RunAsAdmin>false</RunAsAdmin><RedirectRegistry>true</RedirectRegistry><IsAdvancedRedirection>false</IsAdvancedRedirection></Profile></Profiles></LEConfig>"#;

    #[test]
    fn migration_preserves_guids_arguments_and_existing_configs() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("LEConfig.xml");
        let new = temp.path().join("LEPConfig.xml");
        fs::write(&old, LEGACY).unwrap();
        let tool = temp.path().join("LEPProc_x64.exe");
        prepare_locale_config(&tool).unwrap();
        let profiles = locale_profiles(&tool).unwrap();
        assert_eq!(profiles[0].guid, "custom");
        assert_eq!(profiles[0].name, "日本語 & game");
        assert!(!profiles[0].admin);
        let migrated = fs::read_to_string(&new).unwrap();
        assert!(migrated.contains("<Parameter>--name &quot;a b&quot;</Parameter>"));
        assert!(migrated.contains("<RegistryRedirectionMode>1</RegistryRedirectionMode>"));
        assert_eq!(fs::read_to_string(&old).unwrap(), LEGACY);
        fs::write(&old, "broken").unwrap();
        prepare_locale_config(&tool).unwrap();
        assert_eq!(fs::read_to_string(new).unwrap(), migrated);
    }

    #[test]
    fn migration_maps_registry_modes_and_rejects_invalid_flags() {
        for (redirect, advanced, expected) in [("false", "false", 0), ("true", "true", 2)] {
            let text = LEGACY
                .replace(
                    "<RedirectRegistry>true",
                    &format!("<RedirectRegistry>{redirect}"),
                )
                .replace(
                    "<IsAdvancedRedirection>false",
                    &format!("<IsAdvancedRedirection>{advanced}"),
                );
            assert!(super::migrate(&text).unwrap().contains(&format!(
                "<RegistryRedirectionMode>{expected}</RegistryRedirectionMode>"
            )));
        }
        assert!(
            super::migrate(&LEGACY.replace("<RedirectRegistry>true", "<RedirectRegistry>invalid"))
                .is_err()
        );
    }

    #[test]
    fn new_install_has_japanese_profiles_for_both_launchers() {
        let temp = tempfile::tempdir().unwrap();
        let tool = temp.path().join("LEPProc_x86.exe");
        prepare_locale_config(&tool).unwrap();
        let profiles = locale_profiles(&temp.path().join("LEPProc_x64.exe")).unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|p| p.locale == "ja-JP"));
        assert!(profiles.iter().any(|p| p.admin));
        assert!(profiles.iter().any(|p| !p.admin));
    }

    #[test]
    fn invalid_legacy_config_is_not_silently_reset() {
        for invalid in ["broken", "<LEConfig><Profiles/></LEConfig>"] {
            let temp = tempfile::tempdir().unwrap();
            fs::write(temp.path().join("LEConfig.xml"), invalid).unwrap();
            assert!(prepare_locale_config(&temp.path().join("LEPProc_x64.exe")).is_err());
            assert!(!temp.path().join("LEPConfig.xml").exists());
        }
    }

    #[test]
    fn external_legacy_launcher_uses_only_legacy_config() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("LEConfig.xml"), LEGACY).unwrap();
        let tool = temp.path().join("LEProc.exe");
        prepare_locale_config(&tool).unwrap();
        assert_eq!(locale_profiles(&tool).unwrap()[0].guid, "custom");
        assert!(!temp.path().join("LEPConfig.xml").exists());
    }
}
