//! Documentation policy tests for the public security and privacy contract.

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fs, path::Path};

    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
    }

    fn markdown_targets(document: &str) -> BTreeSet<&str> {
        let mut targets = BTreeSet::new();
        for segment in document.split("](").skip(1) {
            let Some(target) = segment.split(')').next() else {
                continue;
            };
            if !target.is_empty()
                && !target.starts_with("http://")
                && !target.starts_with("https://")
            {
                targets.insert(target);
            }
        }
        targets
    }

    fn heading_anchor(line: &str) -> Option<String> {
        let heading = line.strip_prefix('#')?.trim_start_matches('#').trim();
        if heading.is_empty() {
            return None;
        }
        let anchor: String = heading
            .to_ascii_lowercase()
            .chars()
            .filter_map(|character| {
                if character.is_ascii_alphanumeric() || character == '-' || character == ' ' {
                    Some(if character == ' ' { '-' } else { character })
                } else {
                    None
                }
            })
            .collect();
        Some(anchor)
    }

    #[test]
    fn local_documentation_links_resolve() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
        for document_name in ["README.md", "SECURITY.md"] {
            let document = read(document_name);
            for target in markdown_targets(&document) {
                let (relative, fragment) = target
                    .split_once('#')
                    .map_or((target, None), |(path, anchor)| (path, Some(anchor)));
                let destination = if relative.is_empty() {
                    repository.join(document_name)
                } else {
                    repository.join(relative)
                };
                assert!(
                    destination.is_file(),
                    "{document_name} links to missing local file {target:?}"
                );
                if let Some(fragment) = fragment {
                    let contents = fs::read_to_string(&destination).unwrap_or_else(|error| {
                        panic!("could not read {}: {error}", destination.display())
                    });
                    assert!(
                        contents
                            .lines()
                            .filter_map(heading_anchor)
                            .any(|anchor| anchor == fragment),
                        "{document_name} links to missing heading {target:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn runtime_egress_is_explicit_and_bounded() {
        let readme = read("README.md");
        let stats = read("src/stats.rs");
        let config = read("src/config.rs");

        for endpoint in [
            "public-pool.io:21496",
            "https://public-pool.io:40557/api",
            "https://mempool.space/api",
        ] {
            assert!(
                readme.contains(endpoint),
                "README omits runtime endpoint {endpoint}"
            );
        }
        assert!(config.contains("stratum+tcp://public-pool.io:21496"));
        assert!(stats.contains("https://public-pool.io:40557/api"));
        assert!(stats.contains("https://mempool.space/api"));
        assert!(readme.contains("sends no telemetry"));
        assert!(readme.contains("Plaintext"));
    }

    #[test]
    fn image_inventory_matches_build_inputs() {
        let readme = read("README.md");
        let dockerfile = read("Dockerfile");
        for marker in [
            "debian:bookworm-slim",
            "7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587",
            "8da0556cec32819d967734527a8e0f1d8efb0671",
            "docker history --no-trunc miner-local:dev",
            "docker run --rm --entrypoint dpkg miner-local:dev -l",
        ] {
            assert!(
                readme.contains(marker),
                "README image inventory omits {marker}"
            );
        }
        assert!(dockerfile.contains("ARG CPUMINER_COMMIT=8da0556cec32819d967734527a8e0f1d8efb0671"));
        assert!(dockerfile.contains("COPY --from=app-build /app/target/release/miner"));
    }
}
