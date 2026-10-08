/// Policy test: the cpuminer source must be pinned to a full commit SHA and
/// that commit must be verified before compilation. A mutable branch or tag
/// (e.g. `--branch v2.5.1`) is not an acceptable trust anchor — the upstream
/// maintainer can move it after review without anyone noticing.
///
/// This test is intentionally fail-closed: if the Dockerfile cannot be found
/// or parsed, the test fails rather than passing silently.
#[cfg(test)]
mod tests {
    use std::fs;

    fn dockerfile_contents() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir).join("Dockerfile");
        fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
    }

    #[test]
    fn cpuminer_clone_does_not_pin_a_mutable_branch_or_tag() {
        let contents = dockerfile_contents();
        let clone_line = contents
            .lines()
            .find(|l| l.contains("git clone") && l.contains("pooler/cpuminer"))
            .expect("expected a `git clone` line for pooler/cpuminer in the Dockerfile");

        assert!(
            !clone_line.contains("--branch") && !clone_line.contains("--tag"),
            "cpuminer clone must not use a mutable --branch/--tag as the trust anchor: {clone_line}"
        );
    }

    #[test]
    fn cpuminer_commit_arg_is_a_full_commit_sha() {
        let contents = dockerfile_contents();
        let line = contents
            .lines()
            .find(|l| l.trim_start().starts_with("ARG CPUMINER_COMMIT="))
            .expect("expected `ARG CPUMINER_COMMIT=<sha>` default in the Dockerfile");

        let commit = line
            .split_once('=')
            .map(|(_, v)| v.trim())
            .expect("ARG CPUMINER_COMMIT must have a default value");

        assert_eq!(
            commit.len(),
            40,
            "CPUMINER_COMMIT must be a full 40-character commit SHA, got '{commit}' ({} chars)",
            commit.len()
        );
        assert!(
            commit.chars().all(|c| c.is_ascii_hexdigit()),
            "CPUMINER_COMMIT must be hex, got '{commit}'"
        );
    }

    #[test]
    fn build_verifies_checked_out_commit_before_compiling() {
        let contents = dockerfile_contents();

        let checkout_pos = contents
            .find("git checkout \"${CPUMINER_COMMIT}\"")
            .expect("expected an explicit `git checkout \"${CPUMINER_COMMIT}\"` step");
        let verify_pos = contents
            .find("git rev-parse HEAD")
            .expect("expected a commit verification step using `git rev-parse HEAD`");
        let make_pos = contents
            .find("make -j")
            .expect("expected a `make -j...` compilation step for cpuminer");

        assert!(
            checkout_pos < verify_pos && verify_pos < make_pos,
            "the checked-out commit must be verified after checkout and before compilation"
        );
    }

    #[test]
    fn final_image_labels_and_carries_the_verified_commit() {
        let contents = dockerfile_contents();

        assert!(
            contents.contains("io.github.cpuminer.commit="),
            "final image must report its cpuminer source commit via an image label"
        );
        assert!(
            contents.contains("/usr/local/share/cpuminer-commit"),
            "final image must carry the verified commit file for runtime inspection"
        );
    }
}
