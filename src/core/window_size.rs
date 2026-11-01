// The supplied original screenshot uses its 990 x 650 logical-pixel minimum.
pub const MAIN_OUTER_LOGICAL: (f64, f64) = (990., 650.);

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
    fn reference_size_accounts_for_windows_chrome_and_dpi() {
        let inner = main_inner_size(1.5, (1792, 1260), (1770, 1200));
        assert_eq!(inner, (1463, 915));
        assert_eq!((inner.0 + 22, inner.1 + 60), (1485, 975));
        assert_eq!(main_inner_size(1., (1196, 839), (1180, 800)), (974, 611));
    }
}
