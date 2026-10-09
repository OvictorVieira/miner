/// Policy test: every FROM instruction in the Dockerfile must be pinned to an
/// immutable digest (`image:tag@sha256:<hex>`).  A mutable tag alone is not
/// sufficient — even a locked Cargo.lock can be rendered meaningless by a
/// silently re-tagged base image.
///
/// This test is intentionally fail-closed: if the Dockerfile cannot be found or
/// parsed, the test fails rather than passing silently.
#[cfg(test)]
mod tests {
    use std::fs;

    /// Locate the Dockerfile relative to CARGO_MANIFEST_DIR.
    fn dockerfile_contents() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir).join("Dockerfile");
        fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
    }

    /// Returns every external `FROM` image reference found in the file,
    /// excluding previously declared stage aliases, `AS <alias>` suffixes,
    /// and `--platform` flags.
    fn from_references(contents: &str) -> Vec<String> {
        let mut aliases = Vec::new();
        let mut references = Vec::new();

        for line in contents.lines() {
            let trimmed = line.trim();
            if !trimmed.to_uppercase().starts_with("FROM ") {
                continue;
            }
            let rest = trimmed[5..].trim();
            let rest = if rest.starts_with("--") {
                rest.split_once(' ').map(|x| x.1).unwrap_or("").trim()
            } else {
                rest
            };
            let tokens: Vec<_> = rest.split_whitespace().collect();
            let image = tokens.first().copied().unwrap_or("");
            let is_stage_alias = aliases.iter().any(|alias| alias == image);
            if !image.eq_ignore_ascii_case("scratch") && !is_stage_alias {
                references.push(image.to_string());
            }
            if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("AS") {
                aliases.push(tokens[2].to_string());
            }
        }

        references
    }

    #[test]
    fn all_from_images_are_digest_pinned() {
        let contents = dockerfile_contents();
        let references = from_references(&contents);

        assert!(
            !references.is_empty(),
            "no FROM instructions found in Dockerfile — is the path correct?"
        );

        let unpinned: Vec<&str> = references
            .iter()
            .filter(|r| !r.contains("@sha256:"))
            .map(String::as_str)
            .collect();

        assert!(
            unpinned.is_empty(),
            "the following FROM images are not pinned to an immutable digest:\n  {}\n\
             Fix: append @sha256:<hex> to each image reference.",
            unpinned.join("\n  ")
        );
    }

    #[test]
    fn digest_format_is_valid_sha256() {
        let contents = dockerfile_contents();
        let references = from_references(&contents);

        for reference in &references {
            if let Some(digest_part) = reference.split("@sha256:").nth(1) {
                assert_eq!(
                    digest_part.len(),
                    64,
                    "digest for '{reference}' should be 64 hex chars, got {}",
                    digest_part.len()
                );
                assert!(
                    digest_part.chars().all(|c| c.is_ascii_hexdigit()),
                    "digest for '{reference}' contains non-hex characters: '{digest_part}'"
                );
            }
        }
    }

    #[test]
    fn healthcheck_uses_only_the_minimal_health_endpoint() {
        let contents = dockerfile_contents();
        let healthcheck = contents
            .split_once("HEALTHCHECK ")
            .map(|(_, rest)| rest.split("\n\n").next().unwrap_or(rest))
            .expect("Dockerfile must define a HEALTHCHECK");

        assert!(
            healthcheck.contains("http://127.0.0.1:3500/health"),
            "Dockerfile HEALTHCHECK must request only the loopback /health endpoint"
        );
        assert!(
            healthcheck.contains(r#"'{"status":"ok"}'"#),
            "Dockerfile HEALTHCHECK must verify the exact minimal liveness body"
        );
        for forbidden in ["/api/stats", "wallet", "pool", "pid", "restart", "error"] {
            assert!(
                !healthcheck.to_ascii_lowercase().contains(forbidden),
                "Dockerfile HEALTHCHECK contains forbidden detail {forbidden:?}: {healthcheck}"
            );
        }
    }

    #[test]
    fn offline_verification_stage_runs_locked_tests_and_self_test() {
        let contents = dockerfile_contents();
        let verification = contents
            .split_once("FROM app-build AS offline-verification")
            .and_then(|(_, rest)| {
                rest.split_once("Stage 3: final image")
                    .map(|(stage, _)| stage)
            })
            .expect("Dockerfile must define an offline-verification stage before the final image");

        for test_filter in [
            "cargo test --locked payout_identity",
            "cargo test --locked assets_reference_no_third_party_subresources",
        ] {
            assert!(
                verification.contains(test_filter),
                "offline verification must run {test_filter}"
            );
        }
        assert!(
            verification.contains("RUN /app/target/release/miner --self-test"),
            "offline verification must run the release binary self-test"
        );
    }
}
