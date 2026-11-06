//! 日志边界脱敏；不替代调用方禁止传入完整配置、请求和响应的约束。
use regex::Regex;
use std::io;

pub(super) struct Redactor {
    rules: Vec<(Regex, &'static str)>,
}

impl Redactor {
    pub(super) fn new() -> io::Result<Self> {
        let rules = [
            // 保留本地前端堆栈的资源名、行列号，移除查询串与片段。
            (r#"(?i)(?:https?://tauri\.localhost|tauri://localhost)/([^\s?#<>"']+)(?:[?#][^\s<>"']*)?"#, "[app]/$1"),
            (r#"(?i)\b(?:https?|wss?)://[^\s<>"']+"#, "[url]"),
            (r#"(?im)\b(?:authorization|proxy-authorization|cookie|set-cookie)\s*[:=][^\r\n]*"#, "[credential header]"),
            (r#"(?i)\b(?:bearer|basic)\s+[a-z0-9+/_.=~-]+"#, "[credential]"),
            (r#"(?i)["']?(?:[a-z_-]*(?:token|secret|password|passwd|api[_-]?key)|code_verifier|signature)["']?\s*[:=]\s*(?:"[^"\r\n]*"|'[^'\r\n]*'|[^\s,;}]+)"#, "[credential]"),
            (r#"(?i)\b[a-z]:[\\/]+Users[\\/]+[^\\/\s"']+"#, "[user]"),
            (r#"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b"#, "[email]"),
        ].into_iter().map(|(pattern, replacement)| Regex::new(pattern)
            .map(|regex| (regex, replacement)).map_err(io::Error::other))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self { rules })
    }

    pub(super) fn clean(&self, text: &str) -> String {
        let mut result = text.to_owned();
        for (rule, replacement) in &self.rules {
            result = rule.replace_all(&result, *replacement).into_owned();
        }
        // 脱敏之后再截断，避免先截断引号导致凭据匹配失效。
        if let Some((boundary, _)) = result.char_indices().nth(32_768) {
            result.truncate(boundary);
            result.push_str(" [truncated]");
        }
        result
    }
}
