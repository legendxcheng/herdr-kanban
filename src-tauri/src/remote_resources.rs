use crate::{execution::Resources, machines::Machine, remote_transport::request};
use serde_json::json;

impl Resources for Machine {
    fn read_text(&self, path: &str) -> Result<String, String> {
        request(self, json!({"method":"read_text","path":path}))
    }
    fn directory_exists(&self, path: &str) -> Result<bool, String> {
        request(self, json!({"method":"directory_exists","path":path}))
    }
    fn search_files(
        &self,
        root: &str,
        query: &str,
    ) -> Result<Vec<crate::file_search::FileEntry>, String> {
        request(
            self,
            json!({"method":"search_files","root":root,"query":query}),
        )
    }
    fn search_skills(
        &self,
        root: Option<&str>,
        query: &str,
    ) -> Result<Vec<crate::skill_discovery::SkillEntry>, String> {
        request(
            self,
            json!({"method":"search_skills","root":root,"query":query}),
        )
    }
    fn git_statuses(
        &self,
        projects: Vec<(String, String)>,
    ) -> Result<Vec<crate::project_git::ProjectGitStatus>, String> {
        request(self, json!({"method":"git_statuses","projects":projects}))
    }
    fn prepare_image(
        &self,
        _source: &std::path::Path,
        bytes: &[u8],
    ) -> Result<std::path::PathBuf, String> {
        use base64::Engine;
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(bytes));
        let extension = match image::guess_format(bytes).map_err(|_| "image.format")? {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => return Err("image.format".into()),
        };
        let mut query =
            json!({"method":"image","digest":digest,"extension":extension,"size":bytes.len()});
        let existing: Option<std::path::PathBuf> = request(self, &query)?;
        if let Some(path) = existing {
            return Ok(path);
        }
        query["data"] = base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .into();
        let uploaded: Option<std::path::PathBuf> = request(self, query)?;
        uploaded.ok_or_else(|| "image.missing".into())
    }
}
