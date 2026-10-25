//! Generated parsers receive data, never filesystem, shell, network or credentials.
use galgear_i18n::Message;
use rhai::{Dynamic, Engine, Scope, module_resolvers::DummyModuleResolver};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRef {
    pub source: String,
    pub position: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ResumeProfile {
    #[serde(rename = "kag_parser_label_v1")]
    KagParserLabelV1,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ResumeStateRole {
    #[serde(rename = "resume_node")]
    ResumeNode,
}

/// A restore entry independently checked by the Python native-state verifier.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeNode {
    pub profile: ResumeProfile,
    pub state_role: ResumeStateRole,
    pub source: String,
    pub label: String,
    pub position: u64,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Facts {
    /// Verified script source for a dialogue node or independently checked restore entry.
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub anchors: Vec<String>,
    #[serde(default)]
    pub variables: Value,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Save-recorded backlog, independently resolved by the Python evidence verifier.
    #[serde(default)]
    pub history: Vec<NodeRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeNode>,
}

impl Facts {
    pub fn valid_resume(&self) -> bool {
        self.resume.as_ref().is_some_and(|resume| {
            self.source.as_deref() == Some(resume.source.as_str())
                && self.line == Some(resume.position)
                && !resume.source.trim().is_empty()
                && resume.source.len() <= 4096
                && resume.label.starts_with('*')
                && resume.label.len() <= 4096
                && !resume.label.contains(['\r', '\n', '\0'])
                && self.anchors.is_empty()
        })
    }
}

pub fn run(program: &str, input: &Value) -> Result<Facts, Message> {
    if program.len() > 64 * 1024 {
        return Err(Message::new("summary.invalid_program"));
    }
    let mut engine = Engine::new();
    engine.set_module_resolver(DummyModuleResolver::new());
    engine
        .set_max_operations(2_000_000)
        .set_max_call_levels(16)
        .set_max_expr_depths(64, 64)
        .set_max_string_size(128 * 1024)
        .set_max_array_size(20_000)
        .set_max_map_size(128)
        .set_max_modules(0);
    engine.on_print(|_| {}).on_debug(|_, _, _| {});
    let start = Instant::now();
    engine
        .on_progress(move |_| (start.elapsed() > Duration::from_secs(3)).then_some(Dynamic::UNIT));
    engine.register_fn("matches", |text: &str, pattern: &str| -> bool {
        pattern.len() <= 256
            && text.len() <= 32_768
            && regex::Regex::new(pattern).is_ok_and(|r| r.is_match(text))
    });
    let ast = engine
        .compile(program)
        .map_err(|e| Message::detail("summary.invalid_program", e))?;
    let data =
        rhai::serde::to_dynamic(input).map_err(|_| Message::new("summary.invalid_program"))?;
    let result = engine
        .call_fn::<Dynamic>(&mut Scope::new(), &ast, "extract", (data,))
        .map_err(|e| Message::detail("summary.invalid_program", e))?;
    let facts: Facts = rhai::serde::from_dynamic(&result)
        .map_err(|e| Message::detail("summary.invalid_program", e))?;
    if facts.resume.is_some()
        || facts.scene.len() > 1024
        || facts.source.as_ref().is_some_and(|s| s.len() > 4096)
        || facts.anchors.len() > 32
        || facts.anchors.iter().any(|s| s.len() > 4096)
        || facts.warnings.len() > 32
        || serde_json::to_vec(&facts).map_or(true, |b| b.len() > 32_768)
    {
        return Err(Message::new("summary.invalid_program"));
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resume_profiles_and_roles_require_the_native_verifier_contract() {
        let resume = json!({"profile":"kag_parser_label_v1","state_role":"resume_node",
            "source":"chapter.ks","label":"*checkpoint","position":80});
        let value = json!({"source":"chapter.ks","line":80,"anchors":[],"resume":resume});
        let facts: Facts = serde_json::from_value(value.clone()).unwrap();
        assert!(facts.valid_resume());
        for (field, unsupported) in [("profile", "model_defined"), ("state_role", "scene_only")] {
            let mut bad = value.clone();
            bad["resume"][field] = json!(unsupported);
            assert!(serde_json::from_value::<Facts>(bad).is_err());
        }
        let mut bad = facts;
        bad.line = Some(79);
        assert!(!bad.valid_resume());
    }

    #[test]
    fn legacy_rhai_program_cannot_mint_verified_resume_metadata() {
        let program = r#"fn extract(data) { #{ source: "chapter.ks", line: 80, anchors: [], resume: #{ profile: "kag_parser_label_v1", state_role: "resume_node", source: "chapter.ks", label: "*checkpoint", position: 80 } } }"#;
        assert!(run(program, &json!({})).is_err());
    }
    #[test]
    fn parser_cannot_import_files_or_run_forever() {
        assert!(
            run(
                "import \"secret\" as x; fn extract(data) { #{} }",
                &json!({})
            )
            .is_err()
        );
        assert!(run("fn extract(data) { loop {} }", &json!({})).is_err());
    }
    #[test]
    fn generated_parser_extracts_new_values_without_baking_in_a_save() {
        let program = "fn extract(data) { #{ scene: data.strings[0].text, anchors: [data.strings[1].text] } }";
        let facts = run(
            program,
            &json!({"strings":[{"text":"chapter2"},{"text":"Fresh save dialogue"}]}),
        )
        .unwrap();
        assert_eq!(
            (facts.scene.as_str(), facts.anchors[0].as_str()),
            ("chapter2", "Fresh save dialogue")
        );
    }
}
