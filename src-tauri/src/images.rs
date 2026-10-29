//! Immutable local images, draft ownership and durable prompt references.
use crate::{
    models::{Task, TaskPriority},
    tasks,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Write},
    path::PathBuf,
};

const MAX_IMAGE: usize = 10 * 1024 * 1024;
const MAX_TOTAL: u64 = 50 * 1024 * 1024;
const MAX_COUNT: usize = 20;

fn error(e: impl std::fmt::Display) -> String {
    format!("image.storage: {e}")
}

pub fn init(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS image_drafts(
        id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
        task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE);
        CREATE TABLE IF NOT EXISTS prompt_images(
        id TEXT PRIMARY KEY, project_id TEXT NOT NULL, name TEXT NOT NULL, mime TEXT NOT NULL,
        bytes INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL, digest TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS draft_image_refs(
        draft_id TEXT NOT NULL REFERENCES image_drafts(id) ON DELETE CASCADE,
        image_id TEXT NOT NULL REFERENCES prompt_images(id), PRIMARY KEY(draft_id,image_id));
        CREATE TABLE IF NOT EXISTS task_image_refs(
        task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        image_id TEXT NOT NULL REFERENCES prompt_images(id), PRIMARY KEY(task_id,image_id));
        CREATE TABLE IF NOT EXISTS operation_image_refs(
        operation_id TEXT NOT NULL, image_id TEXT NOT NULL REFERENCES prompt_images(id),
        PRIMARY KEY(operation_id,image_id));
        CREATE TABLE IF NOT EXISTS image_prompt_snapshots(
        operation_id TEXT PRIMARY KEY,task_id TEXT NOT NULL,prompt_json TEXT NOT NULL,prompt_text TEXT NOT NULL);
        INSERT OR IGNORE INTO schema_migrations(version) VALUES(9);")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptImage {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

fn metadata(db: &Connection, id: &str) -> Result<(PromptImage, String), String> {
    uuid::Uuid::parse_str(id).map_err(|_| "image.missing")?;
    db.query_row(
        "SELECT id,name,mime,bytes,width,height,digest FROM prompt_images WHERE id=?1",
        [id],
        |r| {
            Ok((
                PromptImage {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    mime: r.get(2)?,
                    bytes: r.get(3)?,
                    width: r.get(4)?,
                    height: r.get(5)?,
                },
                r.get(6)?,
            ))
        },
    )
    .optional()
    .map_err(error)?
    .ok_or_else(|| "image.missing".into())
}

fn path(db: &Connection, image: &PromptImage) -> Result<PathBuf, String> {
    let database = db.path().filter(|p| !p.is_empty()).ok_or("image.storage")?;
    let root = PathBuf::from(database)
        .parent()
        .ok_or("image.storage")?
        .join("images");
    let extension = match image.mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => return Err("image.format".into()),
    };
    Ok(root.join(format!("{}.{}", image.id, extension)))
}

fn read(db: &Connection, id: &str) -> Result<(PromptImage, Vec<u8>), String> {
    let (image, digest) = metadata(db, id)?;
    let file = path(db, &image)?;
    let meta = std::fs::symlink_metadata(&file).map_err(|_| "image.missing")?;
    if !meta.is_file()
        || meta.file_type().is_symlink()
        || meta.len() != image.bytes
        || meta.len() > MAX_IMAGE as u64
    {
        return Err("image.changed".into());
    }
    let bytes = std::fs::read(file).map_err(error)?;
    if format!("{:x}", Sha256::digest(&bytes)) != digest {
        return Err("image.changed".into());
    }
    Ok((image, bytes))
}

pub fn open_draft(db: &Connection, project: &str, task: Option<&str>) -> Result<String, String> {
    if let Some(task) = task {
        tasks::ensure_active(db, task)?;
        let owner: String = db
            .query_row("SELECT project_id FROM tasks WHERE id=?1", [task], |r| {
                r.get(0)
            })
            .map_err(error)?;
        if owner != project {
            return Err("image.owner".into());
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let tx = db.unchecked_transaction().map_err(error)?;
    tx.execute(
        "INSERT INTO image_drafts(id,project_id,task_id) VALUES(?1,?2,?3)",
        params![id, project, task],
    )
    .map_err(error)?;
    if let Some(task) = task {
        tx.execute("INSERT INTO draft_image_refs(draft_id,image_id) SELECT ?1,image_id FROM task_image_refs WHERE task_id=?2",params![id,task]).map_err(error)?;
    }
    tx.commit().map_err(error)?;
    Ok(id)
}

pub struct DecodedUpload {
    image: PromptImage,
    bytes: Vec<u8>,
    digest: String,
}
pub struct PendingUpload {
    decoded: DecodedUpload,
    file: PathBuf,
    draft: String,
    project: String,
    committed: bool,
}

pub fn decode_upload(name: &str, mime: &str, data: &str) -> Result<DecodedUpload, String> {
    if data.len() > MAX_IMAGE.div_ceil(3) * 4 {
        return Err("image.limit".into());
    }
    let bytes = STANDARD.decode(data).map_err(|_| "image.format")?;
    if bytes.len() > MAX_IMAGE {
        return Err("image.limit".into());
    }
    let format = image::guess_format(&bytes).map_err(|_| "image.format")?;
    let expected = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err("image.format".into()),
    };
    if mime != expected {
        return Err("image.format".into());
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|_| "image.format")?;
    if u64::from(width) * u64::from(height) > 40_000_000 {
        return Err("image.limit".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(160 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| "image.format")?;
    let image = PromptImage {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.chars().filter(|c| !c.is_control()).take(160).collect(),
        mime: expected.into(),
        bytes: bytes.len() as u64,
        width,
        height,
    };
    let digest = format!("{:x}", Sha256::digest(&bytes));
    Ok(DecodedUpload {
        image,
        bytes,
        digest,
    })
}

pub fn stage_upload(
    db: &Connection,
    draft: &str,
    decoded: DecodedUpload,
) -> Result<PendingUpload, String> {
    let project = db
        .query_row(
            "SELECT project_id FROM image_drafts WHERE id=?1",
            [draft],
            |r| r.get(0),
        )
        .optional()
        .map_err(error)?
        .ok_or("image.draft_closed")?;
    let file = path(db, &decoded.image)?;
    Ok(PendingUpload {
        decoded,
        file,
        draft: draft.into(),
        project,
        committed: false,
    })
}

impl PendingUpload {
    pub fn write(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.file.parent().ok_or("image.storage")?).map_err(error)?;
        let temporary = self.file.with_extension("part");
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(error)?;
        output.write_all(&self.decoded.bytes).map_err(error)?;
        output.sync_all().map_err(error)?;
        drop(output);
        std::fs::rename(&temporary, &self.file).map_err(error)
    }
    pub fn commit(&mut self, db: &Connection) -> Result<PromptImage, String> {
        let tx = db.unchecked_transaction().map_err(error)?;
        let current: Option<String> = tx
            .query_row(
                "SELECT project_id FROM image_drafts WHERE id=?1",
                [&self.draft],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if current.as_deref() != Some(&self.project) {
            return Err("image.draft_closed".into());
        }
        let image = &self.decoded.image;
        tx.execute("INSERT INTO prompt_images(id,project_id,name,mime,bytes,width,height,digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![image.id,self.project,image.name,image.mime,image.bytes,image.width,image.height,self.decoded.digest]).map_err(error)?;
        tx.execute(
            "INSERT INTO draft_image_refs(draft_id,image_id) VALUES(?1,?2)",
            params![self.draft, image.id],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        self.committed = true;
        Ok(self.decoded.image.clone())
    }
}
impl Drop for PendingUpload {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(self.file.with_extension("part"));
            let _ = std::fs::remove_file(&self.file);
        }
    }
}

#[cfg(test)]
pub fn import_image(
    db: &Connection,
    draft: &str,
    name: &str,
    mime: &str,
    data: &str,
) -> Result<PromptImage, String> {
    let mut pending = stage_upload(db, draft, decode_upload(name, mime, data)?)?;
    pending.write()?;
    pending.commit(db)
}

pub fn preview(db: &Connection, draft: &str, id: &str) -> Result<String, String> {
    let allowed: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM draft_image_refs WHERE draft_id=?1 AND image_id=?2)",
            params![draft, id],
            |r| r.get(0),
        )
        .map_err(error)?;
    if !allowed {
        return Err("image.owner".into());
    }
    let (image, bytes) = read(db, id)?;
    Ok(format!(
        "data:{};base64,{}",
        image.mime,
        STANDARD.encode(bytes)
    ))
}

pub fn document_images(json: &str) -> Result<Vec<String>, String> {
    fn walk(node: &serde_json::Value, ids: &mut Vec<String>) -> Result<(), String> {
        if node["type"] == "promptImage" {
            let id = node["attrs"]["imageId"].as_str().ok_or("image.missing")?;
            uuid::Uuid::parse_str(id).map_err(|_| "image.missing")?;
            ids.push(id.into());
            if ids.len() > MAX_COUNT {
                return Err("image.limit".into());
            }
        }
        if let Some(children) = node["content"].as_array() {
            for child in children {
                walk(child, ids)?;
            }
        }
        Ok(())
    }
    let mut ids = Vec::new();
    if let Ok(doc) = serde_json::from_str(json) {
        walk(&doc, &mut ids)?;
    }
    Ok(ids)
}

#[cfg(test)]
pub fn save_task(
    db: &Connection,
    task: Option<&str>,
    project: &str,
    name: &str,
    json: &str,
    priority: TaskPriority,
    draft: Option<&str>,
) -> Result<Task, String> {
    let tx = db.unchecked_transaction().map_err(error)?;
    let saved = save_task_in(&tx, task, project, name, json, priority, draft)?;
    tx.commit().map_err(error)?;
    Ok(saved)
}

/// The caller owns the transaction, including any accompanying task metadata.
pub(crate) fn save_task_in(
    tx: &Connection,
    task: Option<&str>,
    project: &str,
    name: &str,
    json: &str,
    priority: TaskPriority,
    draft: Option<&str>,
) -> Result<Task, String> {
    let ids = document_images(json)?;
    let mut total = 0;
    if let Some(draft) = draft {
        let owner: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT project_id,task_id FROM image_drafts WHERE id=?1",
                [draft],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(error)?;
        if owner != Some((project.to_string(), task.map(String::from))) {
            return Err("image.owner".into());
        }
    }
    for id in &ids {
        let allowed: bool = if let Some(draft) = draft {
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM draft_image_refs WHERE draft_id=?1 AND image_id=?2)",
                params![draft, id],
                |r| r.get(0),
            )
            .map_err(error)?
        } else {
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM task_image_refs WHERE task_id=?1 AND image_id=?2)",
                params![task, id],
                |r| r.get(0),
            )
            .map_err(error)?
        };
        if !allowed {
            return Err("image.owner".into());
        }
        let (image, _) = read(&tx, id)?;
        total += image.bytes;
    }
    if total > MAX_TOTAL {
        return Err("image.limit".into());
    }
    let saved = if let Some(task) = task {
        tasks::update(&tx, task, name, json, priority)?;
        tasks::list(&tx, project)?
            .into_iter()
            .find(|item| item.id == task)
            .ok_or("task.missing")?
    } else {
        tasks::create(&tx, project, name, priority, json)?
    };
    tx.execute("DELETE FROM task_image_refs WHERE task_id=?1", [&saved.id])
        .map_err(error)?;
    for id in ids {
        tx.execute(
            "INSERT OR IGNORE INTO task_image_refs(task_id,image_id) VALUES(?1,?2)",
            params![saved.id, id],
        )
        .map_err(error)?;
    }
    Ok(saved)
}

pub fn pin_prompt(db: &Connection, task: &str, operation: &str, json: &str) -> Result<(), String> {
    tasks::ensure_active(db, task)?;
    for id in document_images(json)? {
        let allowed: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM task_image_refs WHERE task_id=?1 AND image_id=?2)",
                params![task, id],
                |r| r.get(0),
            )
            .map_err(error)?;
        if !allowed {
            return Err("image.owner".into());
        }
        db.execute(
            "INSERT OR IGNORE INTO operation_image_refs(operation_id,image_id) VALUES(?1,?2)",
            params![operation, id],
        )
        .map_err(error)?;
    }
    Ok(())
}

pub fn verify_operation(db: &Connection, operation: &str) -> Result<(), String> {
    let parent = operation.strip_suffix(":dispatch").unwrap_or(operation);
    let ids: Vec<String> = db
        .prepare("SELECT DISTINCT image_id FROM operation_image_refs WHERE operation_id IN (?1,?2)")
        .map_err(error)?
        .query_map(params![operation, parent], |r| r.get(0))
        .map_err(error)?
        .collect::<Result<_, _>>()
        .map_err(error)?;
    for id in ids {
        read(db, &id)?;
    }
    Ok(())
}

#[cfg(test)]
pub fn prepare_prompt(
    db: &Connection,
    task: &str,
    operation: &str,
    json: &str,
) -> Result<String, String> {
    prepare_prompt_in(db, task, operation, json, &crate::execution::LOCAL)
}

#[cfg(test)]
fn prepare_prompt_in(
    db: &Connection,
    task: &str,
    operation: &str,
    json: &str,
    resources: &dyn crate::execution::Resources,
) -> Result<String, String> {
    let previous:Option<(String,String,String)>=db.query_row("SELECT task_id,prompt_json,prompt_text FROM image_prompt_snapshots WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(error)?;
    if let Some((owner, document, text)) = previous {
        if owner != task || document != json {
            return Err("dispatch.operation_conflict".into());
        }
        return Ok(text);
    }
    let tx = db.unchecked_transaction().map_err(error)?;
    pin_prompt(&tx, task, operation, json)?;
    let prompt = resolve_prompt_in(&tx, json, resources)?;
    tx.execute("INSERT INTO image_prompt_snapshots(operation_id,task_id,prompt_json,prompt_text) VALUES(?1,?2,?3,?4)",params![operation,task,json,prompt]).map_err(error)?;
    tx.commit().map_err(error)?;
    Ok(prompt)
}

impl crate::workflow::Workflow {
    pub fn dispatch_document(
        &self,
        task: &str,
        pane: &str,
        json: &str,
        operation: &str,
    ) -> Result<crate::models::DispatchRecord, String> {
        let epoch = crate::task_reset::epoch(&*self.db.lock().map_err(error)?, task)?;
        let previous = {
            let db = self.db.lock().map_err(error)?;
            tasks::ensure_active(&db, task)?;
            prompt_snapshot(&db, task, operation, json)?
        };
        let prompt = if let Some(prompt) = previous {
            prompt
        } else {
            let (frozen, root_path) = {
                let db = self.db.lock().map_err(error)?;
                let stored: String = db
                    .query_row("SELECT prompt_json FROM tasks WHERE id=?1", [task], |r| {
                        r.get(0)
                    })
                    .map_err(error)?;
                if stored != json {
                    return Err("image.task_changed".into());
                }
                // Frozen bytes survive local GC while SSH runs. Persist references
                // only with a completed snapshot, so failed preparation has no pins.
                let root_path: String = db.query_row(
                    "SELECT p.root_path FROM tasks t JOIN projects p ON p.id=t.project_id WHERE t.id=?1",
                    [task], |r| r.get(0),
                ).map_err(error)?;
                (freeze_document(&db, json)?, root_path)
            };
            // Image uploads cannot block Local database operations.
            let resolved = frozen.resolve(self.herdr.resources(), &root_path)?;
            let db = self.db.lock().map_err(error)?;
            tasks::ensure_active(&db, task)?;
            let stored: String = db
                .query_row("SELECT prompt_json FROM tasks WHERE id=?1", [task], |r| {
                    r.get(0)
                })
                .map_err(error)?;
            if stored != json {
                return Err("image.task_changed".into());
            }
            let tx = db.unchecked_transaction().map_err(error)?;
            crate::task_reset::check_epoch(&tx, task, epoch)?;
            pin_prompt(&tx, task, operation, json)?;
            tx.execute("INSERT OR IGNORE INTO image_prompt_snapshots(operation_id,task_id,prompt_json,prompt_text) VALUES(?1,?2,?3,?4)",params![operation,task,json,resolved]).map_err(error)?;
            tx.commit().map_err(error)?;
            prompt_snapshot(&db, task, operation, json)?.ok_or("image.missing")?
        };
        self.dispatch_at_epoch(task, pane, &prompt, operation, None, epoch)
    }
}

fn prompt_snapshot(
    db: &Connection,
    task: &str,
    operation: &str,
    json: &str,
) -> Result<Option<String>, String> {
    let previous:Option<(String,String,String)>=db.query_row("SELECT task_id,prompt_json,prompt_text FROM image_prompt_snapshots WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(error)?;
    match previous {
        Some((owner, document, text)) if owner == task && document == json => Ok(Some(text)),
        Some(_) => Err("dispatch.operation_conflict".into()),
        None => Ok(None),
    }
}

#[cfg(test)]
pub fn resolve_prompt(db: &Connection, json: &str) -> Result<String, String> {
    resolve_prompt_in(db, json, &crate::execution::LOCAL)
}

#[cfg(test)]
pub fn resolve_prompt_in(
    db: &Connection,
    json: &str,
    resources: &dyn crate::execution::Resources,
) -> Result<String, String> {
    freeze_document(db, json)?.resolve(resources, "")
}

pub struct FrozenDocument {
    doc: serde_json::Value,
    images: std::collections::HashMap<String, (PromptImage, PathBuf, Vec<u8>)>,
}
pub fn freeze_document(db: &Connection, json: &str) -> Result<FrozenDocument, String> {
    let doc = serde_json::from_str(json).map_err(|e| format!("flow.prompt_invalid: {e}"))?;
    let mut images = std::collections::HashMap::new();
    for id in document_images(json)? {
        let (image, bytes) = read(db, &id)?;
        let file = path(db, &image)?;
        images.insert(id, (image, file, bytes));
    }
    Ok(FrozenDocument { doc, images })
}
impl FrozenDocument {
    pub fn resolve(
        &self,
        resources: &dyn crate::execution::Resources,
        root_path: &str,
    ) -> Result<String, String> {
        fn walk(
            images: &std::collections::HashMap<String, (PromptImage, PathBuf, Vec<u8>)>,
            resources: &dyn crate::execution::Resources,
            root_path: &str,
            node: &serde_json::Value,
            text: &mut String,
        ) -> Result<(), String> {
            match node["type"].as_str().unwrap_or("") {
                "text" => text.push_str(node["text"].as_str().unwrap_or("")),
                "promptImage" => {
                    let id = node["attrs"]["imageId"].as_str().ok_or("image.missing")?;
                    let (image, source, bytes) = images.get(id).ok_or("image.missing")?;
                    let file = resources.prepare_image(source, bytes)?;
                    text.push_str(&format!("\n[图片: {}]\n请使用 view_image 读取这张本地原图，再根据图像内容处理任务。原图绝对路径：\n{}\n",image.name,file.display()));
                }
                "mention" => {
                    let label = node["attrs"]["label"].as_str().unwrap_or("");
                    if node["attrs"]["kind"] == "file" {
                        let file = node["attrs"]["ref"].as_str().unwrap_or("");
                        text.push_str(&crate::prompt_reference::file_reference(file, root_path));
                    } else {
                        text.push_str(&format!("${label}"));
                    }
                }
                _ => {
                    if let Some(children) = node["content"].as_array() {
                        for child in children {
                            walk(images, resources, root_path, child, text)?;
                        }
                    }
                }
            }
            if matches!(node["type"].as_str(), Some("paragraph" | "heading")) {
                text.push('\n');
            }
            Ok(())
        }
        let mut text = String::new();
        walk(&self.images, resources, root_path, &self.doc, &mut text)?;
        Ok(text.trim().into())
    }
}

pub fn close_draft(db: &Connection, draft: &str) -> Result<(), String> {
    db.execute("DELETE FROM image_drafts WHERE id=?1", [draft])
        .map_err(error)?;
    schedule_collection(db);
    Ok(())
}

/// Filesystem work uses its own SQLite connection, never the application's Mutex.
pub fn schedule_collection(db: &Connection) {
    let work:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM prompt_images i WHERE NOT EXISTS(SELECT 1 FROM draft_image_refs r WHERE r.image_id=i.id) AND NOT EXISTS(SELECT 1 FROM task_image_refs r WHERE r.image_id=i.id) AND NOT EXISTS(SELECT 1 FROM operation_image_refs r WHERE r.image_id=i.id)) OR EXISTS(SELECT 1 FROM image_prompt_snapshots s WHERE NOT EXISTS(SELECT 1 FROM tasks t WHERE t.id=s.task_id) AND NOT EXISTS(SELECT 1 FROM dispatch_operations d WHERE d.id=s.operation_id))",[],|row|row.get(0)).unwrap_or(false);
    if !work {
        return;
    }
    let Some(file) = db.path().filter(|file| !file.is_empty()).map(PathBuf::from) else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("kanban-image-cleanup".into())
        .spawn(move || {
            if let Ok(connection) =
                Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
            {
                let _ = connection.busy_timeout(std::time::Duration::from_secs(3));
                let _ = connection.execute_batch("PRAGMA foreign_keys=ON");
                let _ = collect_garbage(&connection);
            }
        });
}

pub fn collect_garbage(db: &Connection) -> Result<(), String> {
    release_abandoned_preparations(db, false)?;
    let ids:Vec<String>=db.prepare("SELECT id FROM prompt_images i WHERE NOT EXISTS(SELECT 1 FROM draft_image_refs r WHERE r.image_id=i.id) AND NOT EXISTS(SELECT 1 FROM task_image_refs r WHERE r.image_id=i.id) AND NOT EXISTS(SELECT 1 FROM operation_image_refs r WHERE r.image_id=i.id)").map_err(error)?.query_map([],|r|r.get(0)).map_err(error)?.collect::<Result<_,_>>().map_err(error)?;
    for id in ids {
        let (image, _) = metadata(db, &id)?;
        match std::fs::remove_file(path(db, &image)?) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => continue,
        }
        db.execute("DELETE FROM prompt_images WHERE id=?1", [id])
            .map_err(error)?;
    }
    Ok(())
}

pub fn release_abandoned_preparations(db: &Connection, restarting: bool) -> Result<(), String> {
    let unused = "SELECT s.operation_id FROM image_prompt_snapshots s WHERE (?1 OR NOT EXISTS(SELECT 1 FROM tasks t WHERE t.id=s.task_id)) AND NOT EXISTS(SELECT 1 FROM dispatch_operations d WHERE d.id=s.operation_id) AND NOT EXISTS(SELECT 1 FROM start_requests r WHERE r.operation_id=s.operation_id)";
    let tx = db.unchecked_transaction().map_err(error)?;
    tx.execute(
        &format!("DELETE FROM operation_image_refs WHERE operation_id IN ({unused})"),
        [restarting],
    )
    .map_err(error)?;
    tx.execute(
        &format!("DELETE FROM image_prompt_snapshots WHERE operation_id IN ({unused})"),
        [restarting],
    )
    .map_err(error)?;
    tx.commit().map_err(error)
}

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
