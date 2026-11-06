#[derive(Default)]
pub(crate) struct Trigger {
    last: Option<std::time::Instant>,
}
impl Trigger {
    pub(crate) fn observe(&mut self, pressed: bool, now: std::time::Instant) -> bool {
        // WM_HOTKEY 和 WM_INPUT 可能跨消息轮次到达，编码期间也可能已松键。
        if pressed
            && self
                .last
                .is_none_or(|last| now.duration_since(last).as_millis() >= 250)
        {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Trigger;
    use std::time::{Duration, Instant};
    #[test]
    fn duplicate_input_notifications_only_capture_once() {
        let now = Instant::now();
        let mut trigger = Trigger::default();
        assert!(trigger.observe(true, now));
        assert!(!trigger.observe(true, now + Duration::from_millis(20)));
        assert!(!trigger.observe(false, now + Duration::from_millis(300)));
        assert!(trigger.observe(true, now + Duration::from_millis(400)));
    }
}
