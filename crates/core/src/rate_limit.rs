//! 将 HTTP 限流反馈保留为可匹配消息，由任务调度方决定等待与重试。
use galgear_i18n::Message;
use reqwest::blocking::Response;

/// 识别 429 并保留 Retry-After 秒数；缺少有效秒数时采用 30 秒退避。
pub fn check(response: &Response) -> Result<(), Message> {
    if response.status().as_u16() == 429 {
        let seconds = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| parse_retry_after(value, std::time::SystemTime::now()))
            .unwrap_or(30)
            .max(1);
        return Err(Message::new("sources.rate_limited").arg("seconds", seconds));
    }
    Ok(())
}

/// 返回服务器要求的等待秒数，其他失败不参与自动限流重试。
pub fn retry_after(error: &Message) -> Option<u64> {
    (error.key == "sources.rate_limited").then(|| {
        error
            .args
            .get("seconds")
            .and_then(|value| value.parse().ok())
            .unwrap_or(30)
    })
}

fn parse_retry_after(value: &str, now: std::time::SystemTime) -> Option<u64> {
    if let Ok(seconds) = value.parse() {
        return Some(seconds);
    }
    // RFC 9110 允许秒数或 IMF-fixdate，HTTP 日期按 UTC 解释。
    let fields: Vec<_> = value.split_whitespace().collect();
    if fields.len() != 6 || fields[5] != "GMT" {
        return None;
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|month| *month == fields[2])?
        + 1;
    let date = time::Date::from_calendar_date(
        fields[3].parse().ok()?,
        time::Month::try_from(month as u8).ok()?,
        fields[1].parse().ok()?,
    )
    .ok()?;
    let clock: Vec<_> = fields[4].split(':').collect();
    if clock.len() != 3 {
        return None;
    }
    let deadline = date
        .with_hms(
            clock[0].parse().ok()?,
            clock[1].parse().ok()?,
            clock[2].parse().ok()?,
        )
        .ok()?
        .assume_utc()
        .unix_timestamp();
    let now = now.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((deadline.max(0) as u64).saturating_sub(now))
}

#[cfg(test)]
mod tests {
    use super::parse_retry_after;
    #[test]
    fn retry_after_accepts_seconds_and_http_dates_without_shortening_wait() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(0);
        assert_eq!(parse_retry_after("120", now), Some(120));
        assert_eq!(
            parse_retry_after("Thu, 01 Jan 1970 00:02:00 GMT", now),
            Some(120)
        );
        assert_eq!(parse_retry_after("invalid", now), None);
        assert_eq!(
            parse_retry_after("Thu, 99 Jan 1970 00:02:00 GMT", now),
            None
        );
    }
}
