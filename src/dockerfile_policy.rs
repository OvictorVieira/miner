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

    /// Returns every `FROM` image reference found in the file, excluding the
    /// `AS <alias>` suffix and `--platform` flags.
    fn from_references(contents: &str) -> Vec<String> {
        contents
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                // Match lines that start with FROM (case-insensitive, to be safe)
                if !trimmed.to_uppercase().starts_with("FROM ") {
                    return None;
                }
                // Strip the keyword
                let rest = trimmed[5..].trim();
                // Strip --platform=... flag if present
                let rest = if rest.starts_with("--") {
                    // skip the flag token
                    rest.split_once(' ').map(|x| x.1).unwrap_or("").trim()
                } else {
                    rest
                };
                // Strip the " AS <alias>" suffix
                let image = rest.split_whitespace().next().unwrap_or("").to_string();
                // Ignore the special "scratch" image — it has no digest
                if image.eq_ignore_ascii_case("scratch") {
                    None
                } else {
                    Some(image)
                }
            })
            .collect()
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
}
