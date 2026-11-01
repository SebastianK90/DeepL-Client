// The supplied original screenshot uses its 990 x 650 logical-pixel minimum.
pub const MAIN_OUTER_LOGICAL: (f64, f64) = (990., 650.);

#[derive(Clone, Copy, Debug, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResizeEdge {
    North,
    NorthEast,
    East,
    #[default]
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizeRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(any(target_os = "windows", test))]
pub fn quick_resize_rect(
    initial: ResizeRect,
    delta: (i64, i64),
    edge: ResizeEdge,
    minimum: (i32, i32),
) -> ResizeRect {
    let west = matches!(
        edge,
        ResizeEdge::West | ResizeEdge::NorthWest | ResizeEdge::SouthWest
    );
    let east = matches!(
        edge,
        ResizeEdge::East | ResizeEdge::NorthEast | ResizeEdge::SouthEast
    );
    let north = matches!(
        edge,
        ResizeEdge::North | ResizeEdge::NorthWest | ResizeEdge::NorthEast
    );
    let south = matches!(
        edge,
        ResizeEdge::South | ResizeEdge::SouthWest | ResizeEdge::SouthEast
    );
    let width = if west || east {
        i64::from(initial.width)
            .saturating_add(if west {
                delta.0.saturating_neg()
            } else {
                delta.0
            })
            .clamp(i64::from(minimum.0.max(1)), i64::from(i32::MAX)) as i32
    } else {
        initial.width
    };
    let height = if north || south {
        i64::from(initial.height)
            .saturating_add(if north {
                delta.1.saturating_neg()
            } else {
                delta.1
            })
            .clamp(i64::from(minimum.1.max(1)), i64::from(i32::MAX)) as i32
    } else {
        initial.height
    };
    let x = if west {
        i64::from(initial.x) + i64::from(initial.width) - i64::from(width)
    } else {
        i64::from(initial.x)
    };
    let y = if north {
        i64::from(initial.y) + i64::from(initial.height) - i64::from(height)
    } else {
        i64::from(initial.y)
    };
    ResizeRect {
        x: x.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        y: y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
        width,
        height,
    }
}

pub fn main_inner_size(scale: f64, outer: (u32, u32), inner: (u32, u32)) -> (u32, u32) {
    let frame = (
        outer.0.saturating_sub(inner.0),
        outer.1.saturating_sub(inner.1),
    );
    let target = (
        (MAIN_OUTER_LOGICAL.0 * scale).round().max(1.) as u32,
        (MAIN_OUTER_LOGICAL.1 * scale).round().max(1.) as u32,
    );
    (
        target.0.saturating_sub(frame.0).max(1),
        target.1.saturating_sub(frame.1).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resizing_each_edge_preserves_the_opposite_edge() {
        let r = ResizeRect {
            x: -900,
            y: 120,
            width: 800,
            height: 600,
        };
        let cases = [
            (
                ResizeEdge::North,
                ResizeRect {
                    y: 160,
                    height: 560,
                    ..r
                },
            ),
            (
                ResizeEdge::NorthEast,
                ResizeRect {
                    y: 160,
                    width: 830,
                    height: 560,
                    ..r
                },
            ),
            (ResizeEdge::East, ResizeRect { width: 830, ..r }),
            (
                ResizeEdge::SouthEast,
                ResizeRect {
                    width: 830,
                    height: 640,
                    ..r
                },
            ),
            (ResizeEdge::South, ResizeRect { height: 640, ..r }),
            (
                ResizeEdge::SouthWest,
                ResizeRect {
                    x: -870,
                    width: 770,
                    height: 640,
                    ..r
                },
            ),
            (
                ResizeEdge::West,
                ResizeRect {
                    x: -870,
                    width: 770,
                    ..r
                },
            ),
            (
                ResizeEdge::NorthWest,
                ResizeRect {
                    x: -870,
                    y: 160,
                    width: 770,
                    height: 560,
                },
            ),
        ];
        for (edge, expected) in cases {
            assert_eq!(
                quick_resize_rect(r, (30, 40), edge, (492, 292)),
                expected,
                "{edge:?}"
            );
        }
    }

    #[test]
    fn top_left_resize_stops_at_scaled_client_minimum_with_frame() {
        let r = ResizeRect {
            x: -1000,
            y: 40,
            width: 900,
            height: 700,
        };
        let result = quick_resize_rect(r, (i64::MAX, i64::MAX), ResizeEdge::NorthWest, (754, 454));
        assert_eq!(
            result,
            ResizeRect {
                x: -854,
                y: 286,
                width: 754,
                height: 454
            }
        );
        assert_eq!(result.x + result.width, r.x + r.width);
        assert_eq!(result.y + result.height, r.y + r.height);
    }
    #[test]
    fn reference_size_accounts_for_windows_chrome_and_dpi() {
        let inner = main_inner_size(1.5, (1792, 1260), (1770, 1200));
        assert_eq!(inner, (1463, 915));
        assert_eq!((inner.0 + 22, inner.1 + 60), (1485, 975));
        assert_eq!(main_inner_size(1., (1196, 839), (1180, 800)), (974, 611));
    }
}
