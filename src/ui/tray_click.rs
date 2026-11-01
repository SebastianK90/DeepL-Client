use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};

pub(super) fn opens_main(event: &TrayIconEvent) -> bool {
    // Wait for release so a single physical click opens only once.
    matches!(
        event,
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(button: MouseButton, button_state: MouseButtonState) -> TrayIconEvent {
        TrayIconEvent::Click {
            id: "status".into(),
            position: tauri::PhysicalPosition::new(0., 0.),
            rect: tauri::Rect {
                position: tauri::PhysicalPosition::new(0, 0).into(),
                size: tauri::PhysicalSize::new(16, 16).into(),
            },
            button,
            button_state,
        }
    }

    #[test]
    fn one_left_click_opens_once_on_release_and_other_buttons_do_not() {
        assert!(opens_main(&click(MouseButton::Left, MouseButtonState::Up)));
        assert!(!opens_main(&click(
            MouseButton::Left,
            MouseButtonState::Down
        )));
        for button in [MouseButton::Right, MouseButton::Middle] {
            for state in [MouseButtonState::Down, MouseButtonState::Up] {
                assert!(!opens_main(&click(button, state)));
            }
        }
    }
}
