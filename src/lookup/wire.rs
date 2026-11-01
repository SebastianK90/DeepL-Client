use anyhow::{Result, bail, ensure};
use std::io::Cursor;
pub const MAX_PACKET: usize = 2 * 1024 * 1024;
const INVALID: &str = "Ungültige Antwort des interaktiven Dienstes.";

#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
    Other,
}
pub struct Proto<'a>(Vec<(u32, Field<'a>)>);
fn read_number(data: &[u8], at: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *data.get(*at).ok_or_else(|| anyhow::anyhow!(INVALID))?;
        *at += 1;
        ensure!(shift < 63 || byte <= 1, INVALID);
        value |= ((byte & 127) as u64) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    bail!(INVALID)
}
impl<'a> Proto<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        ensure!(data.len() <= MAX_PACKET, INVALID);
        let mut at = 0;
        let mut fields = Vec::new();
        while at < data.len() {
            ensure!(fields.len() < 20000, INVALID);
            let key = read_number(data, &mut at)?;
            ensure!(key >> 3 > 0 && key >> 3 < 1 << 29, INVALID);
            let tag = (key >> 3) as u32;
            let value = match key & 7 {
                0 => Field::Number(read_number(data, &mut at)?),
                2 => {
                    let len = usize::try_from(read_number(data, &mut at)?)?;
                    let end = at
                        .checked_add(len)
                        .ok_or_else(|| anyhow::anyhow!(INVALID))?;
                    let bytes = data.get(at..end).ok_or_else(|| anyhow::anyhow!(INVALID))?;
                    at = end;
                    Field::Bytes(bytes)
                }
                1 | 5 => {
                    at += if key & 7 == 1 { 8 } else { 4 };
                    ensure!(at <= data.len(), INVALID);
                    Field::Other
                }
                _ => bail!(INVALID),
            };
            fields.push((tag, value));
        }
        Ok(Self(fields))
    }
    pub fn has(&self, tag: u32) -> bool {
        self.0.iter().any(|(k, _)| *k == tag)
    }
    pub fn num(&self, tag: u32) -> u64 {
        self.0
            .iter()
            .find_map(|(k, v)| match v {
                Field::Number(n) if *k == tag => Some(*n),
                _ => None,
            })
            .unwrap_or(0)
    }
    pub fn bytes(&self, tag: u32) -> &'a [u8] {
        self.all(tag).next().unwrap_or(&[])
    }
    pub fn all(&self, tag: u32) -> impl Iterator<Item = &'a [u8]> + '_ {
        self.0.iter().filter_map(move |(k, v)| match v {
            Field::Bytes(b) if *k == tag => Some(*b),
            _ => None,
        })
    }
    pub fn text(&self, tag: u32) -> Result<String> {
        Ok(std::str::from_utf8(self.bytes(tag))
            .map_err(|_| anyhow::anyhow!(INVALID))?
            .to_string())
    }
}
pub fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 127) as u8;
        n >>= 7;
        out.push(byte | if n > 0 { 128 } else { 0 });
        if n == 0 {
            return out;
        }
    }
}
pub fn number(tag: u32, n: u64) -> Vec<u8> {
    [varint((tag as u64) * 8), varint(n)].concat()
}
pub fn bytes(tag: u32, data: &[u8]) -> Vec<u8> {
    [
        varint((tag as u64) * 8 + 2),
        varint(data.len() as u64),
        data.to_vec(),
    ]
    .concat()
}
pub fn string(tag: u32, text: &str) -> Vec<u8> {
    bytes(tag, text.as_bytes())
}
pub fn version(n: u64) -> Vec<u8> {
    bytes(1, &number(1, n))
}
pub fn read_version(data: &[u8]) -> Result<u64> {
    Ok(Proto::parse(Proto::parse(data)?.bytes(1))?.num(1))
}
pub fn frame(value: &rmpv::Value) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    rmpv::encode::write_value(&mut data, value)?;
    ensure!(data.len() <= MAX_PACKET, INVALID);
    Ok([varint(data.len() as u64), data].concat())
}
pub fn take_frame(buffer: &mut Vec<u8>) -> Result<Option<rmpv::Value>> {
    let mut len = 0usize;
    let mut prefix = 0;
    let mut complete = false;
    for byte in buffer.iter().take(5) {
        len |= ((byte & 127) as usize) << (prefix * 7);
        prefix += 1;
        if byte & 128 == 0 {
            complete = true;
            break;
        }
    }
    ensure!(prefix < 5 || complete, INVALID);
    if !complete {
        return Ok(None);
    }
    ensure!(len <= MAX_PACKET, INVALID);
    if buffer.len() < prefix + len {
        return Ok(None);
    }
    let mut cursor = Cursor::new(&buffer[prefix..prefix + len]);
    let value = rmpv::decode::read_value_with_max_depth(&mut cursor, 32)
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    ensure!(cursor.position() as usize == len, INVALID);
    buffer.drain(..prefix + len);
    Ok(Some(value))
}
pub fn utf16_byte(text: &str, offset: u64) -> Result<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Ok(byte);
        }
        units += ch.len_utf16() as u64;
        if units > offset {
            bail!(INVALID);
        }
    }
    ensure!(units == offset, INVALID);
    Ok(text.len())
}
pub fn edit(text: &mut String, start: u64, end: u64, replacement: &str) -> Result<()> {
    ensure!(start <= end, INVALID);
    let a = utf16_byte(text, start)?;
    let b = utf16_byte(text, end)?;
    ensure!(text.len() - (b - a) + replacement.len() <= 100_000, INVALID);
    text.replace_range(a..b, replacement);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protobuf_bounds_and_wrapped_versions() {
        let payload = version(23);
        assert_eq!(
            Proto::parse(Proto::parse(&payload).unwrap().bytes(1))
                .unwrap()
                .num(1),
            23
        );
        for malformed in [&[0x80][..], &[0x0a, 4, 1][..], &[0][..], &[0xff; 12][..]] {
            assert!(Proto::parse(malformed).is_err());
        }
        assert_eq!(Proto::parse(&bytes(9, b"test")).unwrap().bytes(9), b"test");
    }
    #[test]
    fn target_edits_use_utf16_and_reject_split_surrogates() {
        let mut text = "A 🌍 schönes Haus".to_string();
        edit(&mut text, 5, 12, "tolles").unwrap();
        assert_eq!(text, "A 🌍 tolles Haus");
        assert!(edit(&mut text, 3, 4, "x").is_err());
        assert!(edit(&mut text, 20, 21, "x").is_err());
    }
    #[test]
    fn signalr_frames_are_incremental_and_bounded() {
        let packet = frame(&rmpv::Value::Array(vec![6.into()])).unwrap();
        let mut buffer = packet[..1].to_vec();
        assert!(take_frame(&mut buffer).unwrap().is_none());
        buffer.extend(&packet[1..]);
        assert_eq!(
            take_frame(&mut buffer).unwrap().unwrap(),
            rmpv::Value::Array(vec![6.into()])
        );
        assert!(buffer.is_empty());
        assert!(take_frame(&mut vec![0xff; 8]).is_err());
    }
}
