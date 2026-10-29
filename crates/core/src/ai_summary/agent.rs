//! Generic model tool host; all format-specific Python is authored by the selected AI.
use super::{io, read};
use base64::{Engine, engine::general_purpose::STANDARD};
use galgear_i18n::Message;
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

// Keep in sync with the extraction host's verification policies.
const CONTRACT_VERSION: u64 = 4;
// Policy 3 revalidates current/history/resume roles across field proof forms.
pub(super) const SAVE_VERIFICATION_VERSION: u32 = 3;
// Policy 3 revalidates wrapped/encrypted PSB as a complete typed document.
pub(super) const SCRIPT_VERIFICATION_VERSION: u32 = 3;
pub(super) const PREPARATION_TIMEOUT_SECONDS: u64 = 2700;

fn command(home: &Path) -> Result<Command, Message> {
    if let Some(path) = crate::ai::runtime::host(home) {
        return Ok(Command::new(path));
    }
    #[cfg(debug_assertions)]
    {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ai_summary/agent_host.py");
        if source.is_file() {
            let mut cmd = Command::new("python");
            cmd.args(["-I", "-S"]).arg(source);
            return Ok(cmd);
        }
    }
    Err(Message::new("summary.tools_missing"))
}

fn invoke(home: &Path, work: &Path, payload: Value, parse: bool) -> Result<Value, Message> {
    let output = tempfile::NamedTempFile::new_in(work).map_err(io)?;
    let errors = tempfile::NamedTempFile::new_in(work).map_err(io)?;
    let mut command = command(home)?;
    let previous_session_id = if !parse {
        let previous = read(&work.join("session.json"), 32 * 1024)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let previous_session_id = previous
            .as_ref()
            .and_then(|session| session["session_id"].as_str())
            .map(str::to_owned);
        if let Some(previous) = previous {
            super::write_json(&work.join("previous-session.json"), &previous)?;
        }
        super::write_json(
            &work.join("session.json"),
            &json!({"status":"running","closed":false,"started_at":super::now()}),
        )?;
        previous_session_id
    } else {
        None
    };
    if parse {
        command.arg("--parse");
    }
    command
        .stdin(Stdio::piped())
        .stdout(output.reopen().map_err(io)?)
        .stderr(errors.reopen().map_err(io)?);
    #[cfg(windows)]
    let spawned = super::process::spawn(&mut command, work, parse);
    #[cfg(not(windows))]
    let spawned = command.spawn();
    let mut child = spawned.map_err(|error| {
        close_aborted_session(work, parse, "failed");
        io(error)
    })?;
    let bytes = serde_json::to_vec(&payload).map_err(io)?;
    let written = child
        .stdin
        .take()
        .ok_or_else(super::invalid)?
        .write_all(&bytes);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        close_aborted_session(work, parse, "failed");
        return Err(io(error));
    }
    let start = Instant::now();
    let limit = Duration::from_secs(if parse {
        50
    } else {
        PREPARATION_TIMEOUT_SECONDS
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                close_aborted_session(work, parse, "failed");
                return Err(io(error));
            }
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            close_aborted_session(work, parse, "timed_out");
            return Err(Message::detail(
                "summary.agent_failed",
                "Timed out; tool checkpoints are preserved",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if !status.success() {
        close_aborted_session(work, parse, "failed");
        if !parse
            && let Some(message) =
                preparation_unavailable_message(work, previous_session_id.as_deref())
        {
            return Err(message);
        }
        let errors = String::from_utf8_lossy(&read(errors.path(), 1024 * 1024)?).into_owned();
        let detail = errors
            .lines()
            .rev()
            .find(|line| !line.is_empty())
            .unwrap_or("Agent process failed");
        return Err(Message::detail("summary.agent_failed", detail));
    }
    close_aborted_session(work, parse, "completed");
    serde_json::from_slice(&read(output.path(), 64 * 1024 * 1024)?).map_err(io)
}

fn preparation_unavailable_message(
    work: &Path,
    previous_session_id: Option<&str>,
) -> Option<Message> {
    let session: Value =
        serde_json::from_slice(&read(&work.join("session.json"), 32 * 1024).ok()?).ok()?;
    let session_id = session["session_id"].as_str().filter(|id| !id.is_empty())?;
    // An old unavailable report must not relabel a new startup or parser failure.
    if Some(session_id) == previous_session_id
        || session["closed"] != true
        || session["status"] != "unavailable"
        || session["failure"]["kind"] != "missing_progress_save"
    {
        return None;
    }
    Some(Message::new("summary.missing_progress_save"))
}

pub(super) fn close_aborted_session(work: &Path, parse: bool, status: &str) {
    if parse {
        return;
    }
    let path = work.join("session.json");
    let mut session = read(&path, 32 * 1024)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    if session["closed"] == true {
        return;
    }
    // Killing and waiting for the worker closes its sockets even when Python cannot
    // run its finally block. Record that terminal state for the next task.
    session["status"] = json!(status);
    session["closed"] = json!(true);
    session["closed_at"] = json!(super::now());
    let _ = super::write_json(&path, &session);
    if let Some(id) = session["session_id"].as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    }) {
        let archived = work.join("sessions").join(id);
        if let (Ok(root), Ok(directory)) = (work.canonicalize(), archived.canonicalize())
            && directory.starts_with(&root)
        {
            let _ = super::write_json(&directory.join("session.json"), &session);
        }
    }
}

fn validate_preparation_result(result: &Value) -> Result<(), Message> {
    if result["contract_version"].as_u64() != Some(CONTRACT_VERSION)
        || result["save_verification_version"].as_u64()
            != Some(u64::from(SAVE_VERIFICATION_VERSION))
        || result["script_verification_version"].as_u64()
            != Some(u64::from(SCRIPT_VERIFICATION_VERSION))
        || result["speaker_verification"]["version"].as_u64() != Some(1)
        || (result["speaker_verification"]["external_rows"]
            .as_u64()
            .is_some_and(|count| count > 0)
            && result["speaker_verification"]["scope_version"].as_u64() != Some(2))
    {
        return Err(Message::detail(
            "summary.agent_failed",
            "The extraction host is outdated; rebuild or update the bundled AI agent",
        ));
    }
    Ok(())
}

pub fn prepare(home: &Path, work: &Path, config: Value) -> Result<Value, Message> {
    fs::create_dir_all(work).map_err(io)?;
    let result = invoke(home, work, config, false)?;
    validate_preparation_result(&result)?;
    Ok(result)
}

pub fn parse(home: &Path, directory: &Path, path: &Path) -> Result<super::sandbox::Facts, Message> {
    let program =
        String::from_utf8(read(&directory.join("save_extractor.py"), 128 * 1024)?).map_err(io)?;
    let data = read(path, 32 * 1024 * 1024)?;
    if fs::metadata(directory.join("dialogue.jsonl"))
        .map_err(io)?
        .len()
        > 256 * 1024 * 1024
    {
        return Err(super::invalid());
    }
    let dialogue = std::io::BufRead::lines(std::io::BufReader::new(
        fs::File::open(directory.join("dialogue.jsonl")).map_err(io)?,
    ))
    .map(|row| serde_json::from_str::<Value>(&row.map_err(io)?).map_err(io))
    .collect::<Result<Vec<_>, _>>()?;
    if dialogue.len() > 500_000 {
        return Err(super::invalid());
    }
    let known: std::collections::BTreeSet<_> = dialogue
        .iter()
        .filter_map(|row| row["text"].as_str().map(str::to_owned))
        .collect();
    let sources: std::collections::BTreeSet<_> = dialogue
        .iter()
        .filter_map(|row| row["source"].as_str())
        .collect();
    let request = json!({"program":program,"data":STANDARD.encode(data),"verify_save":true,"metadata":{"name":path.file_name().and_then(|s|s.to_str()),"path":crate::config::display_path(path),"dialogue":dialogue}});
    let facts: super::sandbox::Facts =
        serde_json::from_value(invoke(home, directory, request, true)?)
            .map_err(|e| Message::detail("summary.invalid_program", e))?;
    if facts
        .source
        .as_ref()
        .is_none_or(|s| s.is_empty() || s.len() > 4096)
        || facts.line.is_none()
        || (facts.resume.is_some()
            && (!facts.valid_resume()
                || facts
                    .source
                    .as_deref()
                    .is_none_or(|source| !sources.contains(source))))
        || (facts.resume.is_none() && facts.anchors.is_empty())
        || facts.anchors.len() > 32
        || facts.anchors.iter().any(|a| a.len() > 4096)
        || facts.anchors.iter().any(|a| !known.contains(a))
        || serde_json::to_vec(&facts).map_err(io)?.len() > 32768
    {
        return Err(Message::new("summary.invalid_program"));
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_unavailable_session_returns_a_localized_missing_story_save_message() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::write_json(
            &temporary.path().join("session.json"),
            &json!({"session_id":"new-session","closed":true,"status":"unavailable",
                "failure":{"kind":"missing_progress_save"}}),
        )
        .unwrap();
        let message =
            preparation_unavailable_message(temporary.path(), Some("old-session")).unwrap();
        let restored = Message::from_error(&serde_json::to_string(&message).unwrap());
        assert_eq!(restored.key, "summary.missing_progress_save");
        assert_eq!(
            restored.render(galgear_i18n::Language::Chinese),
            "未发现剧情进度存档。请进入游戏剧情，新建存档后重试开启 AI 分析。"
        );
        assert!(
            restored
                .render(galgear_i18n::Language::English)
                .contains("create a new save")
        );
        assert!(
            restored
                .render(galgear_i18n::Language::Japanese)
                .contains("新しくセーブ")
        );
    }

    #[test]
    fn the_previous_unavailable_session_cannot_relabel_a_new_failure() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::write_json(
            &temporary.path().join("session.json"),
            &json!({"session_id":"old-session","closed":true,"status":"unavailable",
                "failure":{"kind":"missing_progress_save"}}),
        )
        .unwrap();
        assert!(preparation_unavailable_message(temporary.path(), Some("old-session")).is_none());
    }

    #[test]
    fn unfinished_unavailable_session_is_not_a_missing_story_save_result() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::write_json(
            &temporary.path().join("session.json"),
            &json!({"session_id":"new-session","closed":false,"status":"unavailable",
                "failure":{"kind":"missing_progress_save"}}),
        )
        .unwrap();
        assert!(preparation_unavailable_message(temporary.path(), None).is_none());
    }

    #[test]
    fn other_terminal_states_keep_the_original_failure_path() {
        let temporary = tempfile::tempdir().unwrap();
        for status in [
            "running",
            "failed",
            "timed_out",
            "budget_exhausted",
            "completed",
        ] {
            super::super::write_json(
                &temporary.path().join("session.json"),
                &json!({"session_id":"new-session","closed":true,"status":status,
                    "failure":{"kind":"missing_progress_save"}}),
            )
            .unwrap();
            assert!(preparation_unavailable_message(temporary.path(), None).is_none());
        }
    }

    #[test]
    fn unsupported_codec_and_format_do_not_claim_a_missing_story_save() {
        let temporary = tempfile::tempdir().unwrap();
        for kind in ["unsupported_codec", "unsupported_format"] {
            super::super::write_json(
                &temporary.path().join("session.json"),
                &json!({"session_id":"new-session","closed":true,"status":"unavailable",
                    "failure":{"kind":kind}}),
            )
            .unwrap();
            assert!(preparation_unavailable_message(temporary.path(), None).is_none());
        }
    }

    #[test]
    fn an_unavailable_file_alone_cannot_relabel_a_new_startup_failure() {
        let temporary = tempfile::tempdir().unwrap();
        super::super::write_json(
            &temporary.path().join("unavailable.json"),
            &json!({"reason":"missing_progress_save"}),
        )
        .unwrap();
        super::super::write_json(
            &temporary.path().join("session.json"),
            &json!({"session_id":"new-session","closed":true,"status":"failed"}),
        )
        .unwrap();
        assert!(preparation_unavailable_message(temporary.path(), Some("old-session")).is_none());
    }

    #[test]
    fn missing_empty_and_invalid_session_ids_cannot_identify_this_invocation() {
        let temporary = tempfile::tempdir().unwrap();
        for id in [json!(null), json!(""), json!(7), json!(false)] {
            super::super::write_json(
                &temporary.path().join("session.json"),
                &json!({"session_id":id,"closed":true,"status":"unavailable",
                    "failure":{"kind":"missing_progress_save"}}),
            )
            .unwrap();
            assert!(preparation_unavailable_message(temporary.path(), None).is_none());
        }
    }

    #[test]
    fn unreadable_session_keeps_the_original_failure_path() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("session.json"), b"not json").unwrap();
        assert!(preparation_unavailable_message(temporary.path(), None).is_none());
    }

    #[test]
    fn preparation_requires_the_current_save_verification_version() {
        let mut result = json!({
            "contract_version":CONTRACT_VERSION,
            "save_verification_version":SAVE_VERIFICATION_VERSION,
            "script_verification_version":SCRIPT_VERIFICATION_VERSION,
            "speaker_verification":{"version":1,"external_rows":4,"scope_version":2}
        });
        assert!(validate_preparation_result(&result).is_ok());
        result
            .as_object_mut()
            .unwrap()
            .remove("save_verification_version");
        assert!(validate_preparation_result(&result).is_err());
        for version in [
            json!(0),
            json!(1),
            json!(2),
            json!(SAVE_VERIFICATION_VERSION + 1),
            json!("3"),
            json!(null),
            json!(true),
        ] {
            result["save_verification_version"] = version;
            assert!(validate_preparation_result(&result).is_err());
        }
    }

    #[test]
    fn preparation_requires_the_current_script_verification_version() {
        let mut result = json!({
            "contract_version":CONTRACT_VERSION,
            "save_verification_version":SAVE_VERIFICATION_VERSION,
            "script_verification_version":SCRIPT_VERIFICATION_VERSION,
            "speaker_verification":{"version":1,"external_rows":4,"scope_version":2}
        });
        assert!(validate_preparation_result(&result).is_ok());
        result
            .as_object_mut()
            .unwrap()
            .remove("script_verification_version");
        assert!(validate_preparation_result(&result).is_err());
        for version in [
            json!(0),
            json!(1),
            json!(2),
            json!(SCRIPT_VERIFICATION_VERSION + 1),
            json!("3"),
            json!(null),
            json!(true),
        ] {
            result["script_verification_version"] = version;
            assert!(validate_preparation_result(&result).is_err());
        }
    }

    #[test]
    fn current_save_verification_does_not_bypass_other_host_contracts() {
        let mut result = json!({
            "contract_version":CONTRACT_VERSION,
            "save_verification_version":SAVE_VERIFICATION_VERSION,
            "script_verification_version":SCRIPT_VERIFICATION_VERSION,
            "speaker_verification":{"version":1,"external_rows":4,"scope_version":2}
        });
        result["speaker_verification"]["scope_version"] = json!(1);
        assert!(validate_preparation_result(&result).is_err());
        result["speaker_verification"]["scope_version"] = json!(2);
        result["contract_version"] = json!(CONTRACT_VERSION - 1);
        assert!(validate_preparation_result(&result).is_err());
    }

    #[test]
    fn aborted_worker_retains_request_counts_and_closes_its_archive() {
        let temporary = tempfile::tempdir().unwrap();
        let work = temporary.path();
        let archive = work.join("sessions/known-session");
        fs::create_dir_all(&archive).unwrap();
        super::super::write_json(
            &work.join("session.json"),
            &json!({"closed":false,"session_id":"known-session","requests_sent":27,"in_flight":true}),
        )
        .unwrap();
        close_aborted_session(work, false, "timed_out");
        let session: Value =
            serde_json::from_slice(&fs::read(work.join("session.json")).unwrap()).unwrap();
        let archived: Value =
            serde_json::from_slice(&fs::read(archive.join("session.json")).unwrap()).unwrap();
        assert_eq!(session["closed"], true);
        assert_eq!(session["requests_sent"], 27);
        assert_eq!(session["status"], "timed_out");
        assert_eq!(session, archived);
        close_aborted_session(work, false, "failed");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(work.join("session.json")).unwrap()).unwrap(),
            session
        );
    }
}
