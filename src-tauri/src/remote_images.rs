//! Content-addressed remote attachments. Incomplete uploads are never published.
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
pub fn store(
    root: &Path,
    digest: &str,
    extension: &str,
    size: u64,
    bytes: Option<&[u8]>,
) -> Result<Option<PathBuf>, String> {
    if !matches!(extension, "png" | "jpg" | "webp") {
        return Err("image.format".into());
    }
    if digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        || size > 10 * 1024 * 1024
    {
        return Err("image.changed".into());
    }
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let file = root.join(format!("{digest}.{extension}"));
    let verify = |path: &Path| -> Result<(), String> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != size {
            return Err("image.changed".into());
        }
        let actual = std::fs::read(path).map_err(|e| e.to_string())?;
        if format!("{:x}", Sha256::digest(actual)) != digest {
            return Err("image.changed".into());
        }
        Ok(())
    };
    if file.exists() {
        verify(&file)?;
        return file.canonicalize().map(Some).map_err(|e| e.to_string());
    }
    let Some(bytes) = bytes else { return Ok(None) };
    if bytes.len() as u64 != size || format!("{:x}", Sha256::digest(bytes)) != digest {
        return Err("image.changed".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "image.format")?;
    if !matches!(
        format,
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
    ) {
        return Err("image.format".into());
    }
    let temp = root.join(format!("{}.partial", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut output = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        output
            .write_all(bytes)
            .and_then(|_| output.sync_all())
            .map_err(|e| e.to_string())?;
        drop(output);
        verify(&temp)?;
        // hard_link publishes atomically without replacing another uploader's result.
        match std::fs::hard_link(&temp, &file) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => verify(&file)?,
            Err(e) => return Err(e.to_string()),
        }
        verify(&file)?;
        file.canonicalize().map(Some).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(temp);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_attachments_require_complete_verified_content_and_reuse_it() {
        let root =
            std::env::temp_dir().join(format!("kanban-remote-images-{}", uuid::Uuid::new_v4()));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let bytes = bytes.into_inner();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let size = bytes.len() as u64;
        assert!(store(&root, &digest, "png", size, None).unwrap().is_none());
        assert!(store(&root, &digest, "png", size, Some(&bytes[..4])).is_err());
        assert!(store(&root, &digest, "png", size, None).unwrap().is_none());
        let file = store(&root, &digest, "png", size, Some(&bytes))
            .unwrap()
            .unwrap();
        assert_eq!(
            store(&root, &digest, "png", size, None).unwrap(),
            Some(file.clone())
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        std::fs::write(&file, b"corrupted").unwrap();
        assert!(store(&root, &digest, "png", size, None).is_err());
        std::fs::remove_file(file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
