//! Bounded, length-prefixed JSON shared by separate discovery and control streams.
use serde_json::Value;
use std::io::{Read, Write};

pub const LIMIT: usize = 20 * 1024 * 1024;

pub fn read(input: &mut impl Read) -> Result<Option<Value>, String> {
    let mut length = [0; 4];
    match input.read(&mut length[..1]).map_err(|e| e.to_string())? {
        0 => return Ok(None),
        _ => input
            .read_exact(&mut length[1..])
            .map_err(|e| e.to_string())?,
    }
    let length = u32::from_le_bytes(length) as usize;
    if length > LIMIT {
        return Err("mention.index_limit".into());
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "machine.invalid_response".into())
}

pub fn write(output: &mut impl Write, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("mention.index_limit".into());
    }
    output
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|_| output.write_all(&bytes))
        .and_then(|_| output.flush())
        .map_err(|e| e.to_string())
}

pub fn serve(
    input: &mut impl Read,
    output: &mut impl Write,
    mut execute: impl FnMut(&Value) -> Value,
) -> Result<(), String> {
    while let Some(request) = read(input)? {
        write(output, &execute(&request))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_requests_share_one_stream_and_frames_are_bounded() {
        let mut bytes = vec![];
        write(&mut bytes, &serde_json::json!({"query":"中文"})).unwrap();
        write(&mut bytes, &serde_json::json!({"query":"next"})).unwrap();
        let mut out = vec![];
        serve(&mut bytes.as_slice(), &mut out, Clone::clone).unwrap();
        let mut reader = out.as_slice();
        assert_eq!(read(&mut reader).unwrap().unwrap()["query"], "中文");
        assert_eq!(read(&mut reader).unwrap().unwrap()["query"], "next");
        assert!(read(&mut reader).unwrap().is_none());
        assert!(read(&mut ((LIMIT + 1) as u32).to_le_bytes().as_slice()).is_err());
        assert!(read(&mut [4, 0, 0, 0, b'{'].as_slice()).is_err());
    }
}
