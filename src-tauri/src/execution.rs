//! Resources belong to the same execution environment as the Herdr adapter.
//! Business workflows never interpret an execution path as a host path themselves.
use crate::{file_search::FileEntry, project_git::ProjectGitStatus, skill_discovery::SkillEntry};
use std::path::{Path, PathBuf};

pub trait Resources: Send + Sync {
    fn read_text(&self, path: &str) -> Result<String, String>;
    fn directory_exists(&self, path: &str) -> Result<bool, String>;
    fn search_files(&self, root: &str, query: &str) -> Result<Vec<FileEntry>, String>;
    fn search_skills(&self, root: Option<&str>, query: &str) -> Result<Vec<SkillEntry>, String>;
    fn git_statuses(
        &self,
        projects: Vec<(String, String)>,
    ) -> Result<Vec<ProjectGitStatus>, String>;
    fn prepare_image(&self, source: &Path, bytes: &[u8]) -> Result<PathBuf, String>;
}

pub struct LocalResources;
pub static LOCAL: LocalResources = LocalResources;

impl Resources for LocalResources {
    fn read_text(&self, path: &str) -> Result<String, String> {
        std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
    }
    fn directory_exists(&self, path: &str) -> Result<bool, String> {
        Ok(Path::new(path).is_dir())
    }
    fn search_files(&self, root: &str, query: &str) -> Result<Vec<FileEntry>, String> {
        crate::file_search::search(root.into(), query.into())
    }
    fn search_skills(&self, root: Option<&str>, query: &str) -> Result<Vec<SkillEntry>, String> {
        crate::skill_discovery::search_skills(query.into(), root.map(str::to_owned))
    }
    fn git_statuses(
        &self,
        projects: Vec<(String, String)>,
    ) -> Result<Vec<ProjectGitStatus>, String> {
        Ok(crate::project_git::read_projects(projects))
    }
    fn prepare_image(&self, source: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
        if std::fs::read(source).map_err(|_| "image.missing")? != bytes {
            return Err("image.changed".into());
        }
        source.canonicalize().map_err(|e| e.to_string())
    }
}

impl crate::workflow::Workflow {
    pub fn resolve_document(&self, document: &str, root_path: &str) -> Result<String, String> {
        let frozen = {
            let db = self.db.lock().map_err(|e| e.to_string())?;
            crate::images::freeze_document(&db, document)?
        };
        frozen.resolve(self.herdr.resources(), root_path)
    }
}
