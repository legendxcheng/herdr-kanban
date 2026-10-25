use serde_json::Value;

pub struct Summary {
    pub fallback: String,
    pub input: String,
}

pub fn summarize(json: &str, locale: &str) -> Result<Summary, String> {
    let doc: Value = serde_json::from_str(json).map_err(|_| "task.content_required")?;
    let mut text = String::new();
    let mut input = String::new();
    let mut references = Vec::new();
    let mut images = false;
    fn walk(
        node: &Value,
        text: &mut String,
        input: &mut String,
        references: &mut Vec<String>,
        images: &mut bool,
    ) {
        match node["type"].as_str() {
            Some("text") => {
                if let Some(s) = node["text"].as_str() {
                    text.push_str(s);
                    input.push_str(s);
                }
            }
            Some("mention") => {
                let a = &node["attrs"];
                if matches!(a["kind"].as_str(), Some("file" | "skill"))
                    && a["ref"].as_str().is_some_and(|r| !r.trim().is_empty())
                {
                    if let Some(label) = a["label"].as_str().filter(|s| !s.trim().is_empty()) {
                        references.push(label.into());
                        input.push_str(label);
                    }
                }
            }
            Some("promptImage") => {
                *images |= node["attrs"]["imageId"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty());
            }
            _ => {
                if let Some(children) = node["content"].as_array() {
                    for child in children {
                        walk(child, text, input, references, images);
                    }
                }
            }
        }
        if matches!(
            node["type"].as_str(),
            Some("paragraph" | "heading" | "codeBlock" | "hardBreak")
        ) {
            text.push('\n');
            input.push('\n');
        }
    }
    walk(&doc, &mut text, &mut input, &mut references, &mut images);
    let first = text
        .lines()
        .find(|s| !s.trim().is_empty())
        .or_else(|| references.first().map(String::as_str));
    if first.is_none() && !images {
        return Err("task.content_required".into());
    }
    let fallback = first
        .map(|s| {
            s.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(40)
                .collect()
        })
        .unwrap_or_else(|| {
            if locale.to_lowercase().starts_with("en") {
                "Image task"
            } else {
                "图片任务"
            }
            .into()
        });
    Ok(Summary {
        fallback,
        input: input.trim().chars().take(2000).collect(),
    })
}
