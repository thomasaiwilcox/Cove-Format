use super::*;

pub(super) fn checked_range(
    offset: u64,
    length: u64,
    total_len: usize,
) -> Result<std::ops::Range<usize>, CoveError> {
    let start = usize::try_from(offset).map_err(|_| CoveError::ArithOverflow)?;
    let len = usize::try_from(length).map_err(|_| CoveError::ArithOverflow)?;
    let end = start.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > total_len {
        return Err(CoveError::BufferTooShort);
    }
    Ok(start..end)
}

pub(super) fn put(buf: &mut [u8], pos: &mut usize, bytes: &[u8]) {
    let end = *pos + bytes.len();
    buf[*pos..end].copy_from_slice(bytes);
    *pos = end;
}

pub(super) fn put_u8(buf: &mut [u8], pos: &mut usize, value: u8) {
    buf[*pos] = value;
    *pos += 1;
}

pub(super) fn put_u16(buf: &mut [u8], pos: &mut usize, value: u16) {
    put(buf, pos, &value.to_le_bytes());
}

pub(super) fn put_u32(buf: &mut [u8], pos: &mut usize, value: u32) {
    put(buf, pos, &value.to_le_bytes());
}

pub(super) fn put_u64(buf: &mut [u8], pos: &mut usize, value: u64) {
    put(buf, pos, &value.to_le_bytes());
}

pub(super) fn put_i64(buf: &mut [u8], pos: &mut usize, value: i64) {
    put(buf, pos, &value.to_le_bytes());
}

pub(super) fn take<'a>(
    bytes: &'a [u8],
    pos: &mut usize,
    len: usize,
) -> Result<&'a [u8], CoveError> {
    let end = pos.checked_add(len).ok_or(CoveError::ArithOverflow)?;
    if end > bytes.len() {
        return Err(CoveError::BufferTooShort);
    }
    let out = &bytes[*pos..end];
    *pos = end;
    Ok(out)
}

pub(super) fn take_array<const N: usize>(
    bytes: &[u8],
    pos: &mut usize,
) -> Result<[u8; N], CoveError> {
    let mut out = [0u8; N];
    out.copy_from_slice(take(bytes, pos, N)?);
    Ok(out)
}

pub(super) fn take_u8(bytes: &[u8], pos: &mut usize) -> Result<u8, CoveError> {
    Ok(take(bytes, pos, 1)?[0])
}

pub(super) fn take_u16(bytes: &[u8], pos: &mut usize) -> Result<u16, CoveError> {
    Ok(u16::from_le_bytes(take_array::<2>(bytes, pos)?))
}

pub(super) fn take_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, CoveError> {
    Ok(u32::from_le_bytes(take_array::<4>(bytes, pos)?))
}

pub(super) fn take_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, CoveError> {
    Ok(u64::from_le_bytes(take_array::<8>(bytes, pos)?))
}

pub(super) fn take_i64(bytes: &[u8], pos: &mut usize) -> Result<i64, CoveError> {
    Ok(i64::from_le_bytes(take_array::<8>(bytes, pos)?))
}
