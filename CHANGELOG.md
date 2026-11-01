# Changelog

## 0.1.2

- Resize Quick Assist from every edge and corner. Windows resizing uses the same focus protection as dragging, so the overlay stays open during the gesture.
- Preserve the current Quick Assist size when reopening, with a minimum size that keeps its controls accessible.
- Show a placement-policy icon in the title bar, with localized tooltips and live settings updates.
- Use XWayland for KDE window placement when available, while preserving Wayland shortcuts, PRIMARY selection and screenshot portals.
- Reapply the selected placement after KDE maps the window, fixing centering after moving and reopening it.
- Read the real KDE pointer through a temporary compositor query and map output coordinates to XWayland, avoiding stale X11 pointer coordinates over native Wayland applications.
- Select the language-menu implementation using the actual window backend. Open above or below according to available space and mirror the search field toward Quick Assist.
- Add regression checks for all resize directions, placement indicators, native KDE positioning and compositor coordinate conversion.
