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
fn staged_worktree_view_preserves_and_restores_changes() {
    let root = git_project("hide-worktree-changes");
    Command::new("git")
        .args([
            "-c",
            "user.name=Canon Test",
            "-c",
            "user.email=canon@example.test",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "staged\n").unwrap();
    Command::new("git")
        .arg("add")
        .arg("README.md")
        .current_dir(&root)
        .output()
        .unwrap();
    fs::write(root.join("README.md"), "unstaged\n").unwrap();
    fs::write(root.join("untracked.txt"), "untracked\n").unwrap();

    {
        let _staged_view = StagedWorktreeView::apply(&root).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("README.md")).unwrap(),
            "staged\n"
        );
        assert!(!root.join("untracked.txt").exists());
    }

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
    let _ = fs::remove_dir_all(root);
}

#[test]
fn staged_worktree_view_drops_original_stash_when_newer_stash_exists() {
    let root = git_project("hide-worktree-newer-stash");
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

    {
        let _staged_view = StagedWorktreeView::apply(&root).unwrap();
        fs::write(root.join("newer.txt"), "newer\n").unwrap();
        let output = Command::new("git")
            .args([
                "stash",
                "push",
                "--keep-index",
                "--include-untracked",
                "-m",
                "newer stash",
            ])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "newer stash failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "unstaged\n"
    );
    assert!(root.join("untracked.txt").exists());
    let output = Command::new("git")
        .args(["stash", "list", "--format=%H"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap().lines().count(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stash_recovery_message_prints_exact_oid_recovery_steps() {
    let root = Path::new("/tmp/project");
    let message = stash_recovery_message(root, "abc123", "restore failed");

    assert!(message.contains("stash abc123"));
    assert!(message.contains("git -C /tmp/project stash apply abc123"));
    assert!(message.contains("drop only the stash whose %H is abc123"));
}

#[cfg(unix)]
#[test]
fn git_stdout_path_preserves_non_utf8_bytes() {
    use std::os::unix::ffi::OsStrExt;

    let path = path_from_git_stdout(vec![b'/', b't', 0xff, b'\n']);

    assert_eq!(path.as_os_str().as_bytes(), &[b'/', b't', 0xff]);
}
