use super::config::{QuickPositionPolicy, SavedPosition};

#[derive(Clone, Copy)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// All inputs are physical pixels. `anchor` is the saved position or cursor position.
pub fn place_quick(
    policy: QuickPositionPolicy,
    anchor: Option<SavedPosition>,
    area: WorkArea,
    size: (u32, u32),
) -> SavedPosition {
    let free_x = (i64::from(area.width) - i64::from(size.0)).max(0);
    let free_y = (i64::from(area.height) - i64::from(size.1)).max(0);
    let center = SavedPosition {
        x: (i64::from(area.x) + free_x / 2) as i32,
        y: (i64::from(area.y) + free_y / 2) as i32,
    };
    let p = match policy {
        QuickPositionPolicy::RememberLast | QuickPositionPolicy::MouseCursor => {
            anchor.unwrap_or(center)
        }
        QuickPositionPolicy::Centered => center,
    };
    SavedPosition {
        x: i64::from(p.x).clamp(i64::from(area.x), i64::from(area.x) + free_x) as i32,
        y: i64::from(p.y).clamp(i64::from(area.y), i64::from(area.y) + free_y) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_position_is_clamped_to_monitor_work_area() {
        let area = WorkArea {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        assert_eq!(
            place_quick(
                QuickPositionPolicy::MouseCursor,
                Some(SavedPosition { x: -900, y: 900 }),
                area,
                (736, 434)
            ),
            SavedPosition { x: -900, y: 606 }
        );
        assert_eq!(
            place_quick(QuickPositionPolicy::MouseCursor, None, area, (736, 434)),
            SavedPosition { x: -1328, y: 303 }
        );
    }
    #[test]
    fn negative_monitor_restore_and_center_fallback() {
        let area = WorkArea {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1040,
        };
        let saved = SavedPosition { x: -1700, y: 120 };
        assert_eq!(
            place_quick(
                QuickPositionPolicy::RememberLast,
                Some(saved),
                area,
                (736, 434)
            ),
            saved
        );
        assert_eq!(
            place_quick(QuickPositionPolicy::Centered, Some(saved), area, (736, 434)),
            SavedPosition { x: -1328, y: 303 }
        );
        assert_eq!(
            place_quick(QuickPositionPolicy::RememberLast, None, area, (736, 434)),
            SavedPosition { x: -1328, y: 303 }
        );
    }
    #[test]
    fn moved_monitor_and_larger_dpi_window_stay_reachable() {
        let area = WorkArea {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        assert_eq!(
            place_quick(
                QuickPositionPolicy::RememberLast,
                Some(SavedPosition { x: -1500, y: 800 }),
                area,
                (1104, 651)
            ),
            SavedPosition { x: 0, y: 69 }
        );
        assert_eq!(
            place_quick(QuickPositionPolicy::Centered, None, area, (1500, 900)),
            SavedPosition { x: 0, y: 0 }
        );
    }
}
