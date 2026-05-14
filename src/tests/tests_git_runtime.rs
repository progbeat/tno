use super::*;

#[test]
fn git_project_root_finds_top_level_from_subdirectory() {
    let root = git_project("git-root-subdir");
    let subdir = root.join(".canon");
    fs::create_dir_all(&subdir).unwrap();
    assert_eq!(
        fs::canonicalize(git_project_root(&subdir).unwrap()).unwrap(),
        fs::canonicalize(&root).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_materializes_staged_snapshot_without_touching_worktree() {
    let root = git_project("staged-snapshot-worktree");
    commit_all(&root, "initial");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "unstaged\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();
    let stash_count_before = stash_count(&root);
    let snapshot_root;

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        snapshot_root = staged_view.snapshot_root().to_path_buf();
        assert_ne!(snapshot_root, root);
        assert_eq!(
            fs::read_to_string(snapshot_root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!snapshot_root.join(".git").exists());
        assert!(!snapshot_root.join("untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "unstaged\n"
        );
        assert!(root.join("untracked.txt").exists());
        assert_eq!(stash_count(&root), stash_count_before);
    }

    assert!(!snapshot_root.exists());
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "unstaged\n"
    );
    assert!(root.join("untracked.txt").exists());
    let diff = Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&diff.stdout).trim(), "README.md");
    assert_eq!(stash_count(&root), stash_count_before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_snapshot_parent_must_be_outside_project_root() {
    let root = git_project("staged-snapshot-parent-outside");
    let root = fs::canonicalize(root).unwrap();
    let inside = root.join("tmp");
    fs::create_dir_all(&inside).unwrap();
    assert!(snapshot_parent_outside_worktree(&root, &root).is_err());
    assert!(snapshot_parent_outside_worktree(&root, &inside).is_err());
    assert!(snapshot_parent_outside_worktree(&root, root.parent().unwrap()).is_ok());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_leaves_ignored_worktree_files_outside_snapshot() {
    let root = git_project("staged-snapshot-ignored");
    fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg(".gitignore")
        .current_dir(&root)
        .output()
        .unwrap();
    commit_all(&root, "ignore file");
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("ignored.txt"), "ignored\n").unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!staged_view.snapshot_root().join("ignored.txt").exists());
    }

    assert_eq!(
        fs::read_to_string(root.join("ignored.txt")).unwrap(),
        "ignored\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
#[cfg(unix)]
fn staged_worktree_view_materializes_literal_pathspec_names_from_index() {
    let root = git_project("staged-snapshot-literal-pathspec");
    commit_all(&root, "initial");
    let special = ":(literal)name.txt";
    fs::write(root.join(special), "staged\n").unwrap();
    let output = Command::new("git")
        .args(["--literal-pathspecs", "add", "--", special])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(root.join(special), "unstaged\n").unwrap();

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(staged_view.snapshot_root().join(special)).unwrap(),
            "staged\n"
        );
    }

    assert_eq!(
        fs::read_to_string(root.join(special)).unwrap(),
        "unstaged\n"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn staged_worktree_view_materializes_symlinks_as_regular_files() {
    use std::os::unix::fs::symlink;

    let root = git_project("staged-snapshot-symlink");
    symlink("/tmp/canon-outside-target", root.join("outside-link")).unwrap();
    let output = Command::new("git")
        .args(["add", "outside-link"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    {
        let staged_view = StagedWorktreeView::apply(&root).unwrap();
        let snapshot_link = staged_view.snapshot_root().join("outside-link");
        let metadata = fs::symlink_metadata(&snapshot_link).unwrap();
        assert!(!metadata.file_type().is_symlink());
        assert!(metadata.file_type().is_file());
        assert_eq!(
            fs::read_to_string(snapshot_link).unwrap(),
            "/tmp/canon-outside-target"
        );
    }

    assert!(fs::symlink_metadata(root.join("outside-link"))
        .unwrap()
        .file_type()
        .is_symlink());
    let _ = fs::remove_dir_all(root);
}

fn stash_count(root: &Path) -> usize {
    let output = Command::new("git")
        .args(["stash", "list", "--format=%H"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().lines().count()
}

#[cfg(unix)]
#[test]
fn git_stdout_path_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path = path_from_git_stdout(vec![b'/', b't', 0xff, b'\n']);

    assert_eq!(path.as_os_str().as_bytes(), &[b'/', b't', 0xff]);
}
