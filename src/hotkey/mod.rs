use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

/// One trigger per pair of distinct C presses; auto-repeat never counts.
pub struct DoubleCopy {
    first: Option<Instant>,
    c_down: bool,
    window: Duration,
}
impl DoubleCopy {
    pub fn new(ms: u64) -> Self {
        Self {
            first: None,
            c_down: false,
            window: Duration::from_millis(ms),
        }
    }
    pub fn event(&mut self, is_c: bool, down: bool, ctrl: bool, now: Instant) -> bool {
        if !is_c {
            if down {
                self.first = None;
            }
            return false;
        }
        if !down {
            self.c_down = false;
            return false;
        }
        if self.c_down {
            return false;
        }
        self.c_down = true;
        if !ctrl {
            self.first = None;
            return false;
        }
        if self
            .first
            .take()
            .is_some_and(|t| now.duration_since(t) <= self.window)
        {
            return true;
        }
        self.first = Some(now);
        false
    }
    pub fn reset(&mut self) {
        self.first = None;
    }
}

pub fn start(app: tauri::AppHandle, ms: u64) {
    #[cfg(target_os = "windows")]
    windows::start(app, ms);
    #[cfg(target_os = "linux")]
    linux::start(app, ms);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeat_timeout_ctrl_and_intervening_keys() {
        let t = Instant::now();
        let mut d = DoubleCopy::new(350);
        assert!(!d.event(true, true, true, t));
        assert!(!d.event(true, true, true, t + Duration::from_millis(80)));
        d.event(true, false, true, t);
        assert!(d.event(true, true, true, t + Duration::from_millis(350)));
        d.event(true, false, true, t);
        assert!(!d.event(true, true, true, t + Duration::from_millis(400)));
        d.event(true, false, true, t);
        assert!(!d.event(true, true, true, t + Duration::from_millis(751)));
        d.event(true, false, true, t);
        d.event(false, true, true, t);
        assert!(!d.event(true, true, true, t + Duration::from_millis(760)));
        d.event(true, false, true, t);
        assert!(!d.event(true, true, false, t + Duration::from_millis(770)));
    }
}
