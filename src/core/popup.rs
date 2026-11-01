use super::placement::WorkArea;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Anchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PopupPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub above: bool,
}

pub fn content_height(rows: usize) -> u32 {
    if rows == 0 {
        104
    } else {
        (64 + rows.min(100) as u32 * 32).min(450)
    }
}

/// Physical-pixel geometry. The popup is a separate native window, not clipped by Quick.
pub fn place_popup(
    anchor: Anchor,
    area: WorkArea,
    desired: (u32, u32),
    gap: u32,
) -> PopupPlacement {
    let left = i64::from(area.x);
    let top = i64::from(area.y);
    let right = left + i64::from(area.width);
    let bottom = top + i64::from(area.height);
    let anchor_top = (anchor.y.round() as i64).clamp(top, bottom);
    let anchor_bottom = ((anchor.y + anchor.height).round() as i64).clamp(top, bottom);
    let above_space = (anchor_top - top - i64::from(gap)).max(0);
    let below_space = (bottom - anchor_bottom - i64::from(gap)).max(0);
    let wanted = i64::from(desired.1);
    let above = below_space < wanted && above_space > below_space;
    let height = wanted
        .min(if above { above_space } else { below_space })
        .max(1)
        .min(i64::from(area.height).max(1));
    let width = i64::from(desired.0).min(i64::from(area.width)).max(1);
    let x = (anchor.x.round() as i64).clamp(left, (right - width).max(left));
    let y = if above {
        anchor_top - i64::from(gap) - height
    } else {
        anchor_bottom + i64::from(gap)
    };
    PopupPlacement {
        x: x as i32,
        y: y.clamp(top, (bottom - height).max(top)) as i32,
        width: width as u32,
        height: height as u32,
        above,
    }
}

#[derive(Default)]
pub struct PopupSession {
    pub revision: u64,
    pub is_open: bool,
    pub resize_sequence: u64,
}
impl PopupSession {
    pub fn open(&mut self) -> u64 {
        self.revision = self.revision.wrapping_add(1).max(1);
        self.is_open = true;
        self.resize_sequence = 0;
        self.revision
    }
    pub fn close(&mut self) {
        self.is_open = false;
        self.revision = self.revision.wrapping_add(1).max(1);
    }
    pub fn accepts(&self, revision: u64) -> bool {
        self.is_open && self.revision == revision
    }
    pub fn accept_resize(&mut self, revision: u64, sequence: u64) -> bool {
        if !self.accepts(revision) || sequence <= self.resize_sequence {
            return false;
        }
        self.resize_sequence = sequence;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_really_extends_below_or_above_the_anchor() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let below = place_popup(
            Anchor {
                x: 100.,
                y: 100.,
                width: 160.,
                height: 40.,
            },
            area,
            (280, 450),
            6,
        );
        assert!(!below.above);
        assert_eq!(
            (below.x, below.y, below.width, below.height),
            (100, 146, 280, 450)
        );
        let above = place_popup(
            Anchor {
                x: 100.,
                y: 850.,
                width: 160.,
                height: 40.,
            },
            area,
            (280, 450),
            6,
        );
        assert!(above.above);
        assert_eq!(above.y + above.height as i32, 844);
        assert_eq!(above.height, 450);
        let filtered = place_popup(
            Anchor {
                x: 100.,
                y: 850.,
                width: 160.,
                height: 40.,
            },
            area,
            (280, content_height(1)),
            6,
        );
        assert!(!filtered.above);
        assert_eq!((filtered.y, filtered.height), (896, 96));
    }
    #[test]
    fn popup_uses_monitor_work_area_and_clamps_horizontal_edges() {
        let area = WorkArea {
            x: -1280,
            y: 0,
            width: 1280,
            height: 720,
        };
        let popup = place_popup(
            Anchor {
                x: -50.,
                y: 350.,
                width: 40.,
                height: 40.,
            },
            area,
            (420, 675),
            9,
        );
        assert!(popup.above);
        assert_eq!(
            (popup.x, popup.y, popup.width, popup.height),
            (-420, 0, 420, 341)
        );
    }
    #[test]
    fn stale_menu_cannot_choose_a_language_after_reopen() {
        let mut session = PopupSession::default();
        let old = session.open();
        assert!(session.accepts(old));
        session.close();
        assert!(!session.accepts(old));
        let new = session.open();
        assert!(!session.accepts(old));
        assert!(session.accepts(new));
    }
    #[test]
    fn delayed_resize_cannot_override_newer_search_or_reopened_menu() {
        let mut session = PopupSession::default();
        let old = session.open();
        assert!(session.accept_resize(old, 2));
        assert!(!session.accept_resize(old, 1));
        assert!(!session.accept_resize(old, 2));
        let new = session.open();
        assert!(!session.accept_resize(old, 3));
        assert!(session.accept_resize(new, 1));
    }
}
