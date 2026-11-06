use std::io::{self, Read};
pub(crate) fn validate(value: &str, len: usize) -> io::Result<()> {
    if value.len() != len
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid engine identifier",
        ));
    }
    Ok(())
}
pub(crate) fn random_hex(bytes: usize) -> io::Result<String> {
    let mut data = vec![0; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut data)?;
    Ok(data.iter().map(|b| format!("{b:02x}")).collect())
}
/// Durable bundle identity is data, not request authority or a pathname.
/// Permit a complete Unix output stem (up to 255 UTF-8 bytes), without aliases.
pub(crate) fn validate_bundle(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid bundle identity",
        ));
    }
    Ok(())
}
pub(crate) fn bundle_hex(value: &str) -> io::Result<String> {
    validate_bundle(value)?;
    Ok(value.bytes().map(|b| format!("{b:02x}")).collect())
}
/// Create and retain this before the first admission attempt. Never log the returned token.
/// Request identity (32 hex digits) is deliberately not mutation authority (256 random bits).
pub fn new_owner_token() -> io::Result<String> {
    random_hex(32)
}
pub fn new_id() -> io::Result<String> {
    random_hex(16)
}
pub(crate) fn equal_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |diff, (x, y)| diff | (x ^ y))
            == 0
}
