//! Physical-pixel selection geometry; independent of monitor origins and DPI.
use anyhow::{Result, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}
impl Region {
    pub fn between(a: Point, b: Point, width: i32, height: i32) -> Option<Self> {
        let region = Self {
            left: a.x.min(b.x).clamp(0, width),
            top: a.y.min(b.y).clamp(0, height),
            right: a.x.max(b.x).clamp(0, width),
            bottom: a.y.max(b.y).clamp(0, height),
        };
        (region.right - region.left >= 3 && region.bottom - region.top >= 3).then_some(region)
    }
    pub fn width(self) -> i32 {
        self.right - self.left
    }
    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
}
pub fn buffer_len(width: i32, height: i32) -> Result<usize> {
    ensure!(
        width > 0 && height > 0 && width <= 30000 && height <= 30000,
        "Screenshot-Erfassung fehlgeschlagen."
    );
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .filter(|p| *p <= 64 * 1024 * 1024)
        .ok_or_else(|| anyhow::anyhow!("Screenshot-Erfassung fehlgeschlagen."))?;
    Ok(pixels * 4)
}
pub fn crop(
    bgra: &[u8],
    width: i32,
    height: i32,
    region: Region,
) -> Result<arboard::ImageData<'static>> {
    ensure!(
        bgra.len() == buffer_len(width, height)?
            && region.left >= 0
            && region.top >= 0
            && region.right <= width
            && region.bottom <= height
            && region.right > region.left
            && region.bottom > region.top,
        "Ungültiger Ziehbereich."
    );
    let mut output = Vec::with_capacity(buffer_len(region.width(), region.height())?);
    for y in region.top..region.bottom {
        let start = ((y as usize) * (width as usize) + region.left as usize) * 4;
        let row = &bgra[start..start + region.width() as usize * 4];
        for p in row.as_chunks::<4>().0 {
            output.extend_from_slice(&[p[2], p[1], p[0], 255]);
        }
    }
    Ok(arboard::ImageData {
        width: region.width() as usize,
        height: region.height() as usize,
        bytes: output.into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_and_outside_drags_are_clamped_in_physical_pixels() {
        let r =
            Region::between(Point { x: 190, y: 110 }, Point { x: -20, y: 10 }, 160, 100).unwrap();
        assert_eq!(
            r,
            Region {
                left: 0,
                top: 10,
                right: 160,
                bottom: 100
            }
        );
        assert!(Region::between(Point { x: 1, y: 1 }, Point { x: 2, y: 80 }, 160, 100).is_none());
        // A negative monitor origin is handled by the native window origin;
        // local selection coordinates remain independent of that origin.
        let origin = Point { x: -1920, y: -200 };
        assert_eq!(origin.x + r.left, -1920);
    }
    #[test]
    fn crop_uses_exact_rows_and_channels_without_touching_source() {
        let source: Vec<u8> = (0..6).flat_map(|n| [n, n + 10, n + 20, 0]).collect();
        let image = crop(
            &source,
            3,
            2,
            Region {
                left: 1,
                top: 0,
                right: 3,
                bottom: 2,
            },
        )
        .unwrap();
        assert_eq!(
            image.bytes.as_ref(),
            &[
                21, 11, 1, 255, 22, 12, 2, 255, 24, 14, 4, 255, 25, 15, 5, 255
            ]
        );
        assert_eq!(source[3], 0);
        assert!(
            crop(
                &source,
                3,
                2,
                Region {
                    left: 0,
                    top: 0,
                    right: 4,
                    bottom: 1
                }
            )
            .is_err()
        );
        assert!(buffer_len(30000, 30000).is_err());
        assert!(buffer_len(-1, 100).is_err());
    }
}
