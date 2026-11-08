//! 按实际采样间隔积累窗口证据，避免把瞬时前台变化当成稳定的游戏归属。
use std::time::{Duration, Instant};

const TIME_CONSTANT_SECONDS: f64 = 2.0;
pub(super) const MAX_SAMPLE_GAP: Duration = Duration::from_secs(3);

pub(super) struct Confidence {
    pub first_seen: Instant,
    pub score: f64,
    last_seen: Instant,
}

impl Confidence {
    pub fn new(now: Instant) -> Self {
        Self {
            first_seen: now,
            last_seen: now,
            score: 0.0,
        }
    }

    pub fn observe(&mut self, evidence: f64, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_seen);
        if elapsed > MAX_SAMPLE_GAP {
            // 休眠或采集失败期间的窗口状态未知，不能用一个新快照补成持续证据。
            self.first_seen = now;
            self.score = 0.0;
        } else {
            let alpha = 1.0 - (-elapsed.as_secs_f64() / TIME_CONSTANT_SECONDS).exp();
            self.score += alpha * (evidence - self.score);
        }
        self.last_seen = now;
    }

    pub fn stable_for(&self, now: Instant, duration: Duration) -> bool {
        now.saturating_duration_since(self.first_seen) >= duration
    }
}
