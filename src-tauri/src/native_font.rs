//! One local font preference shared by all task viewers. WezTerm writes only a
//! private append-only journal; the host alone publishes the active view's size.
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Default)]
pub(super) struct Preferences {
    active: Option<Active>,
}
struct Active {
    id: String,
    journal: PathBuf,
    preference: PathBuf,
    offset: u64,
}

impl Preferences {
    // Reattach an existing viewer without truncating its journal or rebuilding Lua.
    pub(super) fn resume(&mut self, id: &str, directory: &Path) -> Result<(), String> {
        self.flush()?;
        let journal = directory.join("font-events.jsonl");
        let offset = fs::metadata(&journal).map_err(|e| e.to_string())?.len();
        let preference = directory
            .parent()
            .ok_or("terminal.window_timeout")?
            .join("font-preference.json");
        self.active = Some(Active {
            id: id.into(),
            journal,
            preference,
            offset,
        });
        Ok(())
    }
    pub(super) fn activate(
        &mut self,
        id: &str,
        base: &Path,
        view_directory: &Path,
    ) -> Result<String, String> {
        self.flush()?;
        let script = view_directory.join("font.lua");
        let journal = view_directory.join("font-events.jsonl");
        let preference = std::path::absolute(base)
            .map_err(|e| e.to_string())?
            .join("font-preference.json");
        fs::write(&script, include_str!("native_font.lua")).map_err(|e| e.to_string())?;
        File::create(&journal).map_err(|e| e.to_string())?;
        // Lua long strings preserve Unicode and backslashes without JSON's
        // incompatible \u escape syntax. Paths never become shell commands.
        let expression = format!(
            "window_decorations=assert(loadfile({}))(config,{},{})",
            lua_path(&script),
            lua_path(&journal),
            lua_path(&preference),
        );
        self.active = Some(Active {
            id: id.into(),
            journal,
            preference,
            offset: 0,
        });
        Ok(expression)
    }

    pub(super) fn poll(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|a| a.id == id) {
            if let Err(error) = self.flush() {
                eprintln!("Kanban font preference: {error}");
            }
        }
    }

    pub(super) fn finish(&mut self, id: &str) {
        if self.active.as_ref().is_some_and(|a| a.id == id) {
            self.poll(id);
            self.active = None;
        }
    }

    fn flush(&mut self) -> Result<(), String> {
        let Some(active) = self.active.as_mut() else {
            return Ok(());
        };
        let mut file = File::open(&active.journal).map_err(|e| e.to_string())?;
        file.seek(SeekFrom::Start(active.offset))
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(65536)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
            return Ok(());
        };
        let latest = bytes[..=end]
            .split(|b| *b == b'\n')
            .filter_map(|line| serde_json::from_slice::<Option<f64>>(line).ok())
            .filter(|size| size.is_none_or(valid_size))
            .last();
        if let Some(size) = latest {
            let content = serde_json::to_vec(&serde_json::json!({"font_size": size}))
                .map_err(|e| e.to_string())?;
            publish(&active.preference, &content).map_err(|e| e.to_string())?;
        }
        // Retry on write failure; incomplete tail stays unread until the next poll.
        active.offset += (end + 1) as u64;
        Ok(())
    }
}

fn valid_size(size: f64) -> bool {
    size.is_finite() && (6.0..=72.0).contains(&size)
}
fn lua_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut delimiter = String::from("=");
    while text.contains(&format!("]{delimiter}]")) {
        delimiter.push('=');
    }
    format!("[{delimiter}[{text}]{delimiter}]")
}

fn publish(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, content)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};
        let wide = |p: &Path| {
            p.as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>()
        };
        if unsafe {
            MoveFileExW(
                wide(&temporary).as_ptr(),
                wide(path).as_ptr(),
                MOVEFILE_REPLACE_EXISTING,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    fs::rename(temporary, path)
}

#[cfg(test)]
#[path = "native_font_tests.rs"]
mod tests;
