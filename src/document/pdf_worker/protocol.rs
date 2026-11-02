//! Length-prefixed private-pipe protocol; validate type-specific limits before allocation.
use anyhow::{Result, ensure};
use std::io::{Read, Write};
pub(super) const INPUT: u8 = 1;
pub(super) const REQUEST: u8 = 2;
pub(super) const RESPONSE: u8 = 3;
pub(super) const OUTPUT: u8 = 4;
pub(super) const FAILURE: u8 = 5;
pub(super) struct Frame {
    pub kind: u8,
    pub bytes: Vec<u8>,
}
pub(super) fn read(reader: &mut impl Read, allowed: &[u8]) -> Result<Frame> {
    let mut header = [0u8; 5];
    reader.read_exact(&mut header)?;
    let kind = header[0];
    ensure!(allowed.contains(&kind), "Ungültige PDF-Prozessantwort.");
    let len = u32::from_le_bytes(header[1..].try_into()?) as usize;
    let limit = match kind {
        INPUT => 32 * 1024 * 1024,
        REQUEST | RESPONSE => 20_000,
        OUTPUT => 128 * 1024 * 1024,
        FAILURE => 2048,
        _ => 0,
    };
    ensure!(
        len > 0 && len <= limit,
        "PDF-Prozessantwort überschreitet die Größenbegrenzung."
    );
    let mut bytes = vec![0; len];
    reader.read_exact(&mut bytes)?;
    Ok(Frame { kind, bytes })
}
pub(super) fn write(writer: &mut impl Write, frame: &Frame) -> Result<()> {
    let len = u32::try_from(frame.bytes.len())?;
    writer.write_all(&[frame.kind])?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&frame.bytes)?;
    writer.flush()?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_frame_headers_before_reading_a_body() {
        for bytes in [
            [OUTPUT, 255, 255, 255, 255],
            [REQUEST, 0, 0, 0, 0],
            [INPUT, 1, 0, 0, 0],
            [REQUEST, 33, 78, 0, 0],
        ] {
            assert!(read(&mut bytes.as_slice(), &[REQUEST, OUTPUT, FAILURE]).is_err());
        }
    }
    #[test]
    fn preserves_unicode_and_refuses_truncation() {
        let mut wire = Vec::new();
        write(
            &mut wire,
            &Frame {
                kind: REQUEST,
                bytes: "Grüße 世界".as_bytes().to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            read(&mut wire.as_slice(), &[REQUEST]).unwrap().bytes,
            "Grüße 世界".as_bytes()
        );
        wire.pop();
        assert!(read(&mut wire.as_slice(), &[REQUEST]).is_err());
    }
}
