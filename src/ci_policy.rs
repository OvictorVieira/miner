/// Regression tests for the release pipeline's trust boundaries.
///
/// These inspect the checked-in workflow text rather than relying on a live
/// GitHub run, so pull requests fail before an unpinned action, broad token
/// permission, or mutable release tag can reach GitHub-hosted infrastructure.
#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    fn repository_path(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    fn read(relative: &str) -> String {
        let path = repository_path(relative);
        fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
    }

    fn workflows() -> Vec<(String, String)> {
        let directory = repository_path(".github/workflows");
        let mut workflows: Vec<_> = fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
            .map(|entry| {
                entry
                    .expect("workflow directory entry should be readable")
                    .path()
            })
            .filter(|path| {
                matches!(
                    path.extension().and_then(|extension| extension.to_str()),
                    Some("yml" | "yaml")
                )
            })
            .map(|path| {
                let name = path
                    .file_name()
                    .expect("workflow path should have a file name")
                    .to_string_lossy()
                    .into_owned();
                let contents = fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
                (name, contents)
            })
            .collect();
        workflows.sort_by(|left, right| left.0.cmp(&right.0));
        assert!(!workflows.is_empty(), "no GitHub Actions workflows found");
        workflows
    }

    #[test]
    fn every_action_is_pinned_to_a_full_commit_with_a_version_comment() {
        for (name, contents) in workflows() {
            for (index, line) in contents.lines().enumerate() {
                let Some((_, invocation)) = line.trim().split_once("uses:") else {
                    continue;
                };
                let (action, comment) = invocation
                    .trim()
                    .split_once('#')
                    .unwrap_or_else(|| panic!("{name}:{} action lacks version comment", index + 1));
                let reference = action
                    .trim()
                    .rsplit_once('@')
                    .map(|(_, reference)| reference)
                    .unwrap_or_else(|| panic!("{name}:{} action lacks @ reference", index + 1));
                assert!(
                    reference.len() == 40
                        && reference
                            .chars()
                            .all(|character| character.is_ascii_hexdigit()),
                    "{name}:{} action is not pinned to a full commit SHA: {line}",
                    index + 1
                );
                assert!(
                    comment.trim().starts_with('v'),
                    "{name}:{} action comment must identify its reviewed version: {line}",
                    index + 1
                );
            }
        }
    }

    #[test]
    fn permissions_are_read_only_by_default_and_release_only_for_packages() {
        let docker = read(".github/workflows/docker.yml");
        assert!(
            docker.contains("permissions:\n  contents: read\n\njobs:"),
            "Docker workflow must default to contents: read"
        );
        assert_eq!(
            docker.matches("packages: write").count(),
            1,
            "packages: write must occur exactly once"
        );
        let release = docker
            .split_once("  tag-release:")
            .map(|(_, release)| release)
            .expect("Docker workflow must isolate publishing in tag-release job");
        assert!(
            release.contains("packages: write"),
            "only tag-release may receive package write permission"
        );

        let quality = read(".github/workflows/quality.yml");
        assert!(
            quality.contains("permissions:\n  contents: read\n\njobs:"),
            "quality workflow must default to contents: read"
        );
        assert!(
            !quality.contains("write"),
            "quality workflow must be read-only"
        );
    }

    #[test]
    fn ordinary_pushes_and_pull_requests_never_publish_or_read_secrets() {
        let docker = read(".github/workflows/docker.yml");
        let build = docker
            .split_once("  build:")
            .and_then(|(_, jobs)| jobs.split_once("  tag-release:").map(|(build, _)| build))
            .expect("Docker workflow must have separate build and tag-release jobs");
        assert!(
            build.contains("push: false"),
            "ordinary build job must never publish"
        );
        for forbidden in ["login-action", "secrets.", "packages: write", "push: true"] {
            assert!(
                !build.contains(forbidden),
                "ordinary build job contains publishing capability {forbidden:?}"
            );
        }
    }

    #[test]
    fn releases_are_semver_only_without_mutable_aliases_or_deploy_hooks() {
        let docker = read(".github/workflows/docker.yml");
        assert!(
            docker.contains("tags: [\"v[0-9]*.[0-9]*.[0-9]*\"]"),
            "release trigger must prefilter v-prefixed three-part tags"
        );
        assert!(
            docker.contains("^v(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"),
            "release job must validate an exact vMAJOR.MINOR.PATCH SemVer tag"
        );
        assert!(
            docker.contains("tags: type=semver,pattern={{version}}"),
            "release must publish only the complete SemVer image tag"
        );
        for forbidden in [
            "value=latest",
            "pattern={{major}}",
            "pattern={{minor}}",
            "dokploy",
            "webhook",
        ] {
            assert!(
                !docker.to_ascii_lowercase().contains(forbidden),
                "Docker workflow contains forbidden mutable/deploy marker {forbidden:?}"
            );
        }
    }

    #[test]
    fn rust_builds_and_tests_are_locked_and_ci_audits_dependencies() {
        let dockerfile = read("Dockerfile");
        for line in dockerfile
            .lines()
            .filter(|line| line.contains("cargo build"))
        {
            assert!(
                line.contains("--locked"),
                "Docker cargo build must use --locked: {line}"
            );
        }

        let quality = read(".github/workflows/quality.yml");
        for line in quality.lines().filter(|line| line.contains("cargo test")) {
            assert!(
                line.contains("--locked"),
                "CI cargo test must use --locked: {line}"
            );
        }
        assert!(
            quality.contains("run: cargo audit"),
            "quality workflow must run cargo audit"
        );
    }
}
