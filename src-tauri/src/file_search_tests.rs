use super::{FileEntry, search as search_files};
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("kanban-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn search(&self, query: &str) -> Vec<FileEntry> {
        search_files(self.0.to_string_lossy().into_owned(), query.into()).unwrap()
    }

    fn paths(&self, query: &str) -> Vec<String> {
        self.search(query)
            .into_iter()
            .map(|file| file.rel_path)
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ignored_copies_cannot_crowd_out_the_real_source_file() {
    let f = Fixture::new();
    f.write(".gitignore", "/.scratch/\n/release/\n");
    for i in 0..80 {
        f.write(
            &format!(".scratch/copy-{i:02}/MentionPopover.tsx"),
            "temporary",
        );
        f.write(
            &format!("release/copy-{i:02}/MentionPopover.tsx"),
            "generated",
        );
    }
    f.write("kanban/src/MentionPopover.tsx", "source");
    assert_eq!(f.paths("MentionPopover"), ["kanban/src/MentionPopover.tsx"]);
}

#[test]
fn respects_nested_gitignore_and_negation_without_a_git_repository() {
    let f = Fixture::new();
    f.write(".gitignore", "*.log\n!keep.log\n");
    f.write("src/.gitignore", "generated/\n");
    f.write("src/generated/hidden.ts", "");
    f.write("src/source.ts", "");
    f.write("discard.log", "");
    f.write("keep.log", "");
    let found = f.paths("");
    assert!(found.contains(&"src/source.ts".into()));
    assert!(found.contains(&"keep.log".into()));
    assert!(!found.contains(&"discard.log".into()));
    assert!(!found.contains(&"src/generated/hidden.ts".into()));
}

#[test]
fn ranks_every_candidate_before_applying_the_fifty_result_limit() {
    let f = Fixture::new();
    for i in 0..80 {
        f.write(&format!("a-noise/MentionPopover-copy-{i:02}.tsx"), "");
    }
    f.write("z-source/MentionPopover.tsx", "");
    let found = f.search("MentionPopover.tsx");
    // Also exercise a query that matches every copy, not only the source.
    let all = f.paths("MentionPopover");
    assert_eq!(all.len(), 50);
    assert_eq!(all[0], "z-source/MentionPopover.tsx");
    assert_eq!(found[0].rel_path, "z-source/MentionPopover.tsx");
    assert!(found[0].abs_path.ends_with("/z-source/MentionPopover.tsx"));
    assert_eq!(all, f.paths("MentionPopover"));
}

#[test]
fn filename_relevance_precedes_path_matches_and_shorter_paths_break_ties() {
    let f = Fixture::new();
    for path in [
        "nested/deep/App.tsx",
        "src/App.tsx",
        "App.tsx",
        "a/App.tsx.backup",
        "b/MyApp.tsx",
        "App.tsx-directory/other.txt",
    ] {
        f.write(path, "");
    }
    assert_eq!(
        f.paths("APP.TSX"),
        [
            "App.tsx",
            "src/App.tsx",
            "nested/deep/App.tsx",
            "a/App.tsx.backup",
            "b/MyApp.tsx",
            "App.tsx-directory/other.txt",
        ]
    );
}

#[test]
fn supports_relative_paths_windows_separators_and_fuzzy_filenames() {
    let f = Fixture::new();
    f.write("kanban/src/MentionPopover.tsx", "");
    f.write("docs/unrelated.md", "");
    for query in [
        "kanban/src/MentionPopover",
        "./kanban/src/MentionPopover",
        "kanban\\src\\MentionPopover",
        "src/MentionPopover",
        "mnppv",
    ] {
        assert_eq!(f.paths(query), ["kanban/src/MentionPopover.tsx"], "{query}");
    }
}

#[test]
fn explicit_directory_prefix_searches_only_that_ignored_subtree() {
    let f = Fixture::new();
    f.write(".gitignore", "/.scratch/\n/release/\n");
    f.write(".scratch/probe/check.ts", "");
    f.write(".scratch/probe/.gitignore", "ignored.ts\n");
    f.write(".scratch/probe/ignored.ts", "");
    f.write(".scratch/probe/node_modules/noise.ts", "");
    f.write(".scratch/unrelated.ts", "");
    f.write("release/check.ts", "");
    f.write("src/check.ts", "");
    let scoped = f.paths(".scratch/probe/");
    assert!(scoped.contains(&".scratch/probe/check.ts".into()));
    assert!(!scoped.contains(&".scratch/probe/ignored.ts".into()));
    assert!(!scoped.contains(&".scratch/probe/node_modules/noise.ts".into()));
    assert_eq!(f.paths(".scratch/probe/chk"), [".scratch/probe/check.ts"]);
    assert_eq!(f.paths("release/check"), ["release/check.ts"]);
    assert_eq!(f.paths("check.ts"), ["src/check.ts"]);
    for query in ["../", "./../", ".scratch/../src/", "/src/", "C:/src/"] {
        assert!(f.paths(query).is_empty(), "{query}");
    }
}

#[test]
fn prunes_noise_without_hiding_project_dot_directories_or_excluding_the_root() {
    let f = Fixture::new();
    for path in [
        "node_modules/noise.ts",
        ".git/hidden",
        ".scratch/noise.ts",
        "src/target/noise.ts",
        "src/dist/noise.ts",
    ] {
        f.write(path, "");
    }
    for path in [
        ".agents/skills/project/SKILL.md",
        ".github/workflows/build.yml",
        ".codex/config.toml",
        "src/main.ts",
    ] {
        f.write(path, "");
    }
    let found = f.paths("");
    assert_eq!(found.len(), 4);
    assert!(found.contains(&".agents/skills/project/SKILL.md".into()));
    // A project itself named build must still be searchable.
    f.write("build/src/root.ts", "");
    assert_eq!(
        search_files(
            f.0.join("build").to_string_lossy().into_owned(),
            "root".into()
        )
        .unwrap()
        .len(),
        1
    );
}

#[test]
fn rejects_a_missing_or_non_directory_root() {
    let f = Fixture::new();
    f.write("file.txt", "");
    for root in [f.0.join("missing"), f.0.join("file.txt")] {
        assert!(search_files(root.to_string_lossy().into_owned(), "".into()).is_err());
    }
}
