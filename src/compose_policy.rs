//! Policy tests for `docker-compose.yml`: enforce the least-privilege container
//! settings required by the security contract.
//!
//! These tests are text-level checks over the compose file. They intentionally
//! avoid pulling a YAML parser dependency — the goal is to catch structural
//! regressions (e.g. someone adds `privileged: true` or `network: host`), not
//! to validate schema. Comments inside the file are ignored for presence
//! checks so reviewers can leave "never do X" reminders without tripping the
//! guard.

#[cfg(test)]
mod tests {
    use std::fs;

    fn compose_contents() -> String {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let path = std::path::Path::new(manifest_dir).join("docker-compose.yml");
        fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
    }

    /// Returns every non-comment, non-empty line, trimmed of its leading
    /// indentation. Trailing `# ...` comments on live lines are also stripped
    /// so key/value matches don't accidentally hit commented-out directives.
    fn active_lines(contents: &str) -> Vec<String> {
        contents
            .lines()
            .map(|l| {
                // strip inline comments
                match l.find('#') {
                    Some(i) => &l[..i],
                    None => l,
                }
            })
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn user_is_non_root_numeric() {
        let lines = active_lines(&compose_contents());
        let user_line = lines
            .iter()
            .find(|l| l.starts_with("user:"))
            .expect("docker-compose.yml must set `user:` to a non-root numeric UID:GID");
        // Expect user: "<uid>:<gid>" with both > 0.
        let value = user_line
            .trim_start_matches("user:")
            .trim()
            .trim_matches(|c| c == '"' || c == '\'');
        let (uid, gid) = value
            .split_once(':')
            .unwrap_or_else(|| panic!("user must be in UID:GID form, got {value:?}"));
        let uid: u32 = uid
            .parse()
            .unwrap_or_else(|_| panic!("UID must be numeric, got {uid:?}"));
        let gid: u32 = gid
            .parse()
            .unwrap_or_else(|_| panic!("GID must be numeric, got {gid:?}"));
        assert!(uid > 0, "UID must not be root (0)");
        assert!(gid > 0, "GID must not be root (0)");
    }

    #[test]
    fn root_filesystem_is_read_only() {
        let lines = active_lines(&compose_contents());
        assert!(
            lines.iter().any(|l| l == "read_only: true"),
            "docker-compose.yml must set `read_only: true` on the miner service"
        );
    }

    #[test]
    fn drops_all_capabilities() {
        let contents = compose_contents();
        let lines = active_lines(&contents);
        let drop_idx = lines
            .iter()
            .position(|l| l.starts_with("cap_drop:"))
            .expect("docker-compose.yml must declare `cap_drop:`");
        // Expect a list item `- ALL` to follow (allow other items too).
        let follow = &lines[drop_idx + 1..];
        assert!(
            follow.iter().any(|l| l.eq_ignore_ascii_case("- ALL")),
            "cap_drop must include `ALL`"
        );
        // Also forbid any cap_add anywhere in the file for the MVP.
        assert!(
            !lines.iter().any(|l| l.starts_with("cap_add:")),
            "cap_add must not be used in the MVP"
        );
    }

    #[test]
    fn no_new_privileges_is_set() {
        let contents = compose_contents();
        let lines = active_lines(&contents);
        let idx = lines
            .iter()
            .position(|l| l.starts_with("security_opt:"))
            .expect("docker-compose.yml must declare `security_opt:`");
        let follow = &lines[idx + 1..];
        assert!(
            follow
                .iter()
                .any(|l| l.replace(' ', "") == "-no-new-privileges:true"),
            "security_opt must include `no-new-privileges:true`"
        );
    }

    #[test]
    fn seccomp_default_is_never_disabled() {
        let contents = compose_contents();
        // Comment-stripped: catches `seccomp=unconfined` or `seccomp:unconfined`
        // appearing as an active directive anywhere in the file.
        for line in active_lines(&contents) {
            let normalized = line.to_ascii_lowercase().replace(' ', "");
            assert!(
                !normalized.contains("seccomp=unconfined"),
                "seccomp=unconfined is forbidden (line: {line:?})"
            );
            assert!(
                !normalized.contains("seccomp:unconfined"),
                "seccomp:unconfined is forbidden (line: {line:?})"
            );
        }
    }

    #[test]
    fn privileged_mode_absent() {
        let contents = compose_contents();
        for line in active_lines(&contents) {
            // Any active `privileged: true` is forbidden.
            let normalized = line.to_ascii_lowercase().replace(' ', "");
            assert!(
                !normalized.starts_with("privileged:true"),
                "privileged mode must never be enabled (line: {line:?})"
            );
        }
    }

    #[test]
    fn host_namespaces_absent() {
        let contents = compose_contents();
        for line in active_lines(&contents) {
            let normalized = line.to_ascii_lowercase().replace(' ', "");
            for forbidden in ["pid:host", "ipc:host", "network:host", "userns_mode:host"] {
                assert!(
                    !normalized.starts_with(forbidden),
                    "host namespace must not be shared (line: {line:?})"
                );
            }
        }
    }

    #[test]
    fn no_named_volumes_or_bind_mounts() {
        let contents = compose_contents();
        let lines = active_lines(&contents);
        // Top-level `volumes:` block or service-level `volumes:` key — neither
        // is allowed in the MVP. The writable area is a bounded tmpfs only.
        assert!(
            !lines.iter().any(|l| l.starts_with("volumes:")),
            "no named volumes or bind mounts allowed in the MVP (use tmpfs instead)"
        );
    }

    #[test]
    fn tmpfs_is_bounded_and_safe() {
        let contents = compose_contents();
        let lines = active_lines(&contents);
        let idx = lines
            .iter()
            .position(|l| l.starts_with("tmpfs:"))
            .expect("docker-compose.yml must declare a bounded tmpfs for writable scratch");
        let follow = &lines[idx + 1..];
        // At least one tmpfs entry.
        let entries: Vec<&String> = follow.iter().take_while(|l| l.starts_with("- ")).collect();
        assert!(
            !entries.is_empty(),
            "tmpfs list must have at least one entry"
        );
        for entry in entries {
            let lower = entry.to_ascii_lowercase();
            assert!(
                lower.contains("size="),
                "tmpfs entry must have an explicit size= bound: {entry:?}"
            );
            assert!(
                lower.contains("noexec"),
                "tmpfs entry must set noexec: {entry:?}"
            );
            assert!(
                lower.contains("nosuid"),
                "tmpfs entry must set nosuid: {entry:?}"
            );
            assert!(
                lower.contains("nodev"),
                "tmpfs entry must set nodev: {entry:?}"
            );
        }
    }
}
