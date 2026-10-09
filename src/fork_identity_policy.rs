//! Repository policy tests that keep this fork self-contained.

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    const REMOVED_PUBLISHER: &str = concat!("full", "system");
    const REMOVED_DONATION_ADDRESS: &str = concat!(
        "bc1pwy2ulg769ffvhwchk4yzkcq5",
        "yq699qwrrkg3a4lq942rj47sutcq2xjny5"
    );

    fn tracked_files(repo: &Path) -> Vec<String> {
        let output = Command::new("git")
            .args([
                "-C",
                repo.to_str().expect("UTF-8 repository path"),
                "ls-files",
                "-z",
            ])
            .output()
            .expect("git must be available for repository policy tests");
        assert!(
            output.status.success(),
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8(path.to_vec()).expect("tracked path must be UTF-8"))
            .collect()
    }

    #[test]
    fn tracked_files_do_not_restore_retired_publisher_identity() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut violations = Vec::new();

        for relative in tracked_files(repo) {
            let path = repo.join(&relative);
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
            let text = String::from_utf8_lossy(&bytes);
            if text.to_ascii_lowercase().contains(REMOVED_PUBLISHER)
                || text.contains(REMOVED_DONATION_ADDRESS)
            {
                violations.push(relative);
            }
        }

        assert!(
            violations.is_empty(),
            "retired publisher identity found in tracked files: {}",
            violations.join(", ")
        );
    }
}
