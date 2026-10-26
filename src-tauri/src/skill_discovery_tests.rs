use super::search_skills_in;
use std::path::{Path, PathBuf};

#[test]
fn complete_index_can_match_skills_beyond_first_thirty() {
    let f = Fixture::new();
    for i in 0..50 {
        f.write(
            &format!("project/.agents/skills/skill-{i:02}"),
            &format!("skill-{i:02}"),
            "test",
        );
    }
    let all = super::discover_in(Some(&f.0.join("project")), None);
    assert_eq!(all.len(), 50);
    assert_eq!(super::filter(&all, "skill-49")[0].name, "skill-49");
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("kanban-skills-{}", uuid::Uuid::new_v4())))
    }

    fn write(&self, relative: &str, name: &str, description: &str) -> PathBuf {
        let path = self.0.join(relative).join("SKILL.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("---\nname: {name}\ndescription: {description}\n---\nBody"),
        )
        .unwrap();
        path
    }

    fn search(&self, query: &str) -> Vec<super::SkillEntry> {
        search_skills_in(
            Some(&self.0.join("project")),
            Some(&self.0.join("home")),
            query,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[test]
fn finds_project_skill_and_does_not_leak_it_into_another_project() {
    let f = Fixture::new();
    let path = f.write(
        "project/.agents/skills/kanban-dev",
        "kanban-dev",
        "研究和实现 Kanban",
    );
    let found = f.search("KANBAN-DEV");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].skill_md_path, normalized(&path));
    assert_eq!(found[0].description, "研究和实现 Kanban");
    assert_eq!(f.search("研究").len(), 1);
    assert!(
        search_skills_in(
            Some(&f.0.join("other-project")),
            Some(&f.0.join("home")),
            "kanban-dev"
        )
        .is_empty()
    );
    // Project discovery must also work on a machine without a home directory.
    assert_eq!(
        search_skills_in(Some(&f.0.join("project")), None, "kanban-dev").len(),
        1
    );
}

#[test]
fn includes_user_agents_codex_and_legacy_plugin_skills_without_a_project() {
    let f = Fixture::new();
    f.write("home/.agents/skills/agent", "agent", "agent skill");
    f.write(
        "home/.codex/skills/.system/system",
        "system",
        "system skill",
    );
    f.write(
        "home/.claude/plugins/cache/plugin/skills/legacy",
        "legacy",
        "legacy skill",
    );
    f.write("home/unrelated/hidden", "unrelated", "must not be scanned");
    let found = search_skills_in(None, Some(&f.0.join("home")), "");
    assert_eq!(
        found.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["agent", "system", "legacy"]
    );
    assert!(search_skills_in(None, None, "").is_empty());
}

#[test]
fn project_copy_wins_before_filtering_and_limiting() {
    let f = Fixture::new();
    let project = f.write(
        "project/.agents/skills/z-project",
        "z-project",
        "project description",
    );
    f.write(
        "home/.agents/skills/z-project",
        "z-project",
        "shadowed description",
    );
    f.write("home/.codex/skills/z-project", "z-project", "other copy");
    for i in (0..40).rev() {
        f.write(
            &format!("home/.agents/skills/a-{i:02}"),
            &format!("a-{i:02}"),
            "global",
        );
    }
    let found = f.search("");
    assert_eq!(found.len(), 30);
    assert_eq!(found[0].skill_md_path, normalized(&project));
    assert_eq!(found[1].name, "a-00");
    assert_eq!(found[29].name, "a-28");
    assert_eq!(f.search("z-project").len(), 1);
    assert!(f.search("shadowed").is_empty());
    // Filtering is before the cap, even for a match beyond the first 30 entries.
    assert_eq!(f.search("a-39")[0].name, "a-39");
}

#[test]
fn overlapping_roots_and_same_name_copies_produce_one_stable_result() {
    let f = Fixture::new();
    let first = f.write("home/.agents/skills/a-first", "shared", "winner");
    f.write("home/.agents/skills/z-second", "shared", "other copy");
    f.write("home/.codex/skills/shared", "shared", "codex copy");
    let found = search_skills_in(Some(&f.0.join("home")), Some(&f.0.join("home")), "shared");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].skill_md_path, normalized(&first));
}

#[test]
fn missing_roots_bad_files_and_skill_named_directories_do_not_hide_valid_skills() {
    let f = Fixture::new();
    let invalid = f.write("project/.agents/skills/invalid", "invalid", "broken");
    std::fs::write(invalid, [0xff, 0xfe]).unwrap();
    std::fs::create_dir_all(f.0.join("project/.agents/skills/directory/SKILL.md")).unwrap();
    let fallback = f.write("project/.agents/skills/folder-name", "", "");
    std::fs::write(fallback, "A skill without metadata").unwrap();
    let found = f.search("");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "folder-name");
    assert!(found[0].description.is_empty());
}
