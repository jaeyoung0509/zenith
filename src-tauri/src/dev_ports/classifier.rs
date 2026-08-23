use std::path::{Path, PathBuf};

pub struct ProcessClassificationInput<'a> {
    pub pid: u32,
    pub uid: Option<u32>,
    pub current_user_uid: u32,
    pub zenith_pid: u32,
    pub port: u16,
    pub raw_command: &'a str,
    pub process_name: &'a str,
    pub exe_path: Option<&'a Path>,
    pub cwd: Option<&'a Path>,
    pub argv: &'a [String],
    pub started_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationResult {
    pub server_name: String,
    pub project_name: Option<String>,
    pub working_directory: Option<String>,
    pub can_release: bool,
    pub blocked_reason: Option<String>,
}

/// Classifies a process listening on a port according to safety rules and positive dev-server signatures.
pub fn classify_listener(input: &ProcessClassificationInput) -> ClassificationResult {
    let (project_name, working_directory) = sanitize_project_context(input.cwd, input.argv);

    // 1. Privileged port check (ports < 1024)
    if input.port < 1024 {
        return ClassificationResult {
            server_name: clean_process_display_name(input.process_name, input.raw_command),
            project_name,
            working_directory,
            can_release: false,
            blocked_reason: Some("Privileged system port (below 1024)".to_string()),
        };
    }

    // 2. PID checks (0, 1, or Zenith's own PID)
    if input.pid == 0 || input.pid == 1 || input.pid == input.zenith_pid {
        return ClassificationResult {
            server_name: if input.pid == input.zenith_pid {
                "Zenith".to_string()
            } else {
                "System Core".to_string()
            },
            project_name,
            working_directory,
            can_release: false,
            blocked_reason: Some("Zenith or macOS system core process".to_string()),
        };
    }

    // 3. User / Ownership checks
    if let Some(uid) = input.uid {
        if uid == 0 {
            return ClassificationResult {
                server_name: clean_process_display_name(input.process_name, input.raw_command),
                project_name,
                working_directory,
                can_release: false,
                blocked_reason: Some("Root-owned system process".to_string()),
            };
        }
        if uid != input.current_user_uid {
            return ClassificationResult {
                server_name: clean_process_display_name(input.process_name, input.raw_command),
                project_name,
                working_directory,
                can_release: false,
                blocked_reason: Some("Owned by another user".to_string()),
            };
        }
    }

    // Releasing requires stable fields that can be compared immediately before
    // signaling. Missing identity data must never be treated as an allowlisted
    // development server.
    if input.uid.is_none()
        || input.started_at.is_none_or(|started_at| started_at == 0)
        || input.exe_path.is_none()
    {
        return ClassificationResult {
            server_name: clean_process_display_name(input.process_name, input.raw_command),
            project_name,
            working_directory,
            can_release: false,
            blocked_reason: Some("Process identity is unavailable".to_string()),
        };
    }

    // 4. Protected process / system / terminal / database / container checks
    if is_protected_process(input.process_name, input.raw_command, input.exe_path) {
        return ClassificationResult {
            server_name: clean_process_display_name(input.process_name, input.raw_command),
            project_name,
            working_directory,
            can_release: false,
            blocked_reason: Some(
                "Protected system, terminal, database, or container process".to_string(),
            ),
        };
    }

    // 5. Positive dev server signature match
    if let Some(dev_server_name) = match_positive_dev_server_signature(
        input.process_name,
        input.raw_command,
        input.exe_path,
        input.argv,
    ) {
        return ClassificationResult {
            server_name: dev_server_name,
            project_name,
            working_directory,
            can_release: true,
            blocked_reason: None,
        };
    }

    // 6. Fallback: recognized user process but not a recognized development server
    ClassificationResult {
        server_name: clean_process_display_name(input.process_name, input.raw_command),
        project_name,
        working_directory,
        can_release: false,
        blocked_reason: Some("Not recognized as a development server".to_string()),
    }
}

/// Checks if a process belongs to protected categories (shells, terminals, system daemons, databases, container engines).
fn is_protected_process(process_name: &str, raw_cmd: &str, exe_path: Option<&Path>) -> bool {
    let name_lower = process_name.to_ascii_lowercase();
    let cmd_lower = raw_cmd.to_ascii_lowercase();
    let exe_name = exe_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let matches_any = |targets: &[&str]| {
        targets.iter().any(|t| {
            name_lower == *t
                || cmd_lower == *t
                || exe_name == *t
                || name_lower.starts_with(&format!("{t}."))
        })
    };

    // Shells & remote access
    const SHELLS_AND_SSH: &[&str] = &[
        "sh",
        "bash",
        "zsh",
        "fish",
        "csh",
        "tcsh",
        "dash",
        "nu",
        "xonsh",
        "ssh",
        "sshd",
        "mosh-server",
        "mosh-client",
        "tmux",
        "screen",
    ];
    if matches_any(SHELLS_AND_SSH) {
        return true;
    }

    // Terminal applications
    const TERMINALS: &[&str] = &[
        "terminal",
        "iterm2",
        "iterm",
        "alacritty",
        "kitty",
        "ghostty",
        "wezterm-gui",
        "wezterm",
        "warp",
        "hyper",
        "rio",
    ];
    if matches_any(TERMINALS) {
        return true;
    }

    // System daemons & macOS services
    const SYSTEM_DAEMONS: &[&str] = &[
        "launchd",
        "systemd",
        "loginwindow",
        "securityagent",
        "coreauthd",
        "sudo",
        "su",
        "windowserver",
        "mds",
        "mdworker",
        "opendirectoryd",
        "syslogd",
        "notifyd",
        "configd",
        "diskarbitrationd",
        "distnoted",
        "cfprefsd",
        "rapportd",
        "controlcenter",
        "universalaccessd",
        "sharingd",
        "finder",
        "dock",
        "systemsettings",
    ];
    if matches_any(SYSTEM_DAEMONS) {
        return true;
    }

    // Databases & message brokers
    const DATABASES: &[&str] = &[
        "postgres",
        "postmaster",
        "mysqld",
        "mariadbd",
        "mongod",
        "redis-server",
        "memcached",
        "clickhouse",
        "clickhouse-server",
        "cockroach",
        "surreal",
        "etcd",
        "consul",
        "minio",
        "rabbitmq-server",
        "beam.smp",
    ];
    if matches_any(DATABASES) {
        return true;
    }

    // Container runtimes & hypervisors
    const CONTAINERS: &[&str] = &[
        "dockerd",
        "containerd",
        "colima",
        "qemu-system-aarch64",
        "qemu-system-x86_64",
        "com.docker.backend",
        "com.docker.hyperkit",
        "vpnkit",
        "podman",
        "k3s",
        "minikube",
    ];
    if matches_any(CONTAINERS) {
        return true;
    }

    false
}

/// Matches positive signatures for local development servers.
fn match_positive_dev_server_signature(
    process_name: &str,
    raw_cmd: &str,
    exe_path: Option<&Path>,
    argv: &[String],
) -> Option<String> {
    let name_lower = process_name.to_ascii_lowercase();
    let cmd_lower = raw_cmd.to_ascii_lowercase();
    let exe_name = exe_path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    // Check argv lowercase tokens
    let argv_joined = argv.join(" ").to_ascii_lowercase();

    // 1. Vite & SvelteKit
    if argv_mentions_tool(argv, "vite")
        || name_lower == "vite"
        || cmd_lower == "vite"
        || exe_name == "vite"
    {
        if argv_joined.contains("svelte-kit") || argv_joined.contains("@sveltejs/kit") {
            return Some("SvelteKit".to_string());
        }
        return Some("Vite".to_string());
    }

    // 2. Next.js
    if argv_joined.contains("next dev")
        || argv_joined.contains("next-dev")
        || argv_joined.contains("next/dist/bin/next")
        || name_lower == "next-server"
    {
        return Some("Next.js".to_string());
    }

    // 3. Astro
    if argv_has_token_pair(argv, "astro", "dev")
        || argv_mentions_tool(argv, "astro")
        || name_lower == "astro"
        || cmd_lower == "astro"
    {
        return Some("Astro".to_string());
    }

    // 4. Nuxt
    if argv_mentions_tool(argv, "nuxi")
        || argv_has_token_pair(argv, "nuxt", "dev")
        || argv_mentions_tool(argv, "nuxt")
        || name_lower == "nuxt"
    {
        return Some("Nuxt".to_string());
    }

    // 5. Remix
    if argv_joined.contains("remix dev") || argv_joined.contains("@remix-run") {
        return Some("Remix".to_string());
    }

    // 6. Webpack Dev Server
    if argv_joined.contains("webpack-dev-server")
        || argv_joined.contains("webpack serve")
        || (argv_joined.contains("webpack") && argv_joined.contains("serve"))
    {
        return Some("webpack dev server".to_string());
    }

    // 7. Parcel
    if argv_mentions_tool(argv, "parcel")
        || name_lower == "parcel"
        || cmd_lower == "parcel"
        || exe_name == "parcel"
    {
        return Some("Parcel".to_string());
    }

    // 8. Turbopack / Turborepo
    if argv_joined.contains("turbo dev") || argv_joined.contains("turbopack") {
        return Some("Turbopack".to_string());
    }

    // 9. Angular CLI
    if argv_joined.contains("@angular/cli") || argv_joined.contains("ng serve") {
        return Some("Angular CLI".to_string());
    }

    // 10. Wrangler (Cloudflare Workers dev)
    if argv_joined.contains("wrangler") && argv_joined.contains("dev") {
        return Some("Wrangler Dev".to_string());
    }

    // 11. Live-reload tooling (Nodemon, ts-node-dev, tsx watch, esbuild serve)
    if argv_joined.contains("ts-node-dev") {
        return Some("ts-node-dev".to_string());
    }
    if argv_joined.contains("nodemon") {
        return Some("nodemon".to_string());
    }
    if argv_joined.contains("tsx watch") || argv_joined.contains("tsx dev") {
        return Some("tsx dev".to_string());
    }
    if argv_joined.contains("esbuild") && argv_joined.contains("--serve") {
        return Some("esbuild serve".to_string());
    }

    // 12. Bun & Deno dev servers
    if (name_lower == "bun" || cmd_lower == "bun" || exe_name == "bun")
        && (argv_has_exact_token(argv, "dev")
            || argv_joined.contains("--watch")
            || argv_joined.contains("--hot")
            || argv_has_exact_token(argv, "serve"))
    {
        return Some("Bun Dev Server".to_string());
    }
    if (name_lower == "deno" || cmd_lower == "deno" || exe_name == "deno")
        && (argv_joined.contains("task dev")
            || argv_has_exact_token(argv, "serve")
            || argv_joined.contains("--watch"))
    {
        return Some("Deno Dev Server".to_string());
    }

    // 13. Python Dev Servers
    if name_lower.starts_with("python")
        || cmd_lower.starts_with("python")
        || exe_name.starts_with("python")
    {
        if argv_joined.contains("-m http.server") || argv_joined.contains("http.server") {
            return Some("Python http.server".to_string());
        }
        if argv_joined.contains("uvicorn") {
            return Some("Uvicorn".to_string());
        }
        if argv_joined.contains("flask")
            && (argv_joined.contains("run") || argv_joined.contains("dev"))
        {
            return Some("Flask Dev Server".to_string());
        }
        if argv_joined.contains("manage.py") && argv_joined.contains("runserver") {
            return Some("Django Dev Server".to_string());
        }
        if argv_joined.contains("fastapi")
            && (argv_joined.contains("dev") || argv_joined.contains("run"))
        {
            return Some("FastAPI Dev Server".to_string());
        }
        if argv_joined.contains("streamlit") && argv_joined.contains("run") {
            return Some("Streamlit".to_string());
        }
        if argv_joined.contains("gradio") {
            return Some("Gradio".to_string());
        }
    }

    // Uvicorn standalone executable
    if name_lower == "uvicorn" || cmd_lower == "uvicorn" || exe_name == "uvicorn" {
        return Some("Uvicorn".to_string());
    }

    // 14. Go / Rust dev reloaders
    if name_lower == "air" || cmd_lower == "air" || exe_name == "air" {
        return Some("Air (Go Dev)".to_string());
    }
    if argv_joined.contains("cargo-watch")
        || (argv_joined.contains("cargo") && argv_joined.contains("watch"))
    {
        return Some("Cargo Watch".to_string());
    }
    if argv_joined.contains("trunk serve")
        || (argv_joined.contains("trunk") && argv_joined.contains("serve"))
    {
        return Some("Trunk (Rust WASM)".to_string());
    }

    None
}

fn argv_has_exact_token(argv: &[String], expected: &str) -> bool {
    argv.iter()
        .any(|argument| argument.eq_ignore_ascii_case(expected))
}

fn argv_has_token_pair(argv: &[String], first: &str, second: &str) -> bool {
    argv.windows(2)
        .any(|pair| pair[0].eq_ignore_ascii_case(first) && pair[1].eq_ignore_ascii_case(second))
}

fn argv_mentions_tool(argv: &[String], tool: &str) -> bool {
    argv.iter().any(|argument| {
        argument
            .split(['/', '\\'])
            .any(|component| matches_tool_component(component, tool))
    })
}

fn matches_tool_component(component: &str, tool: &str) -> bool {
    component.eq_ignore_ascii_case(tool)
        || ["js", "mjs", "cjs"]
            .iter()
            .any(|extension| component.eq_ignore_ascii_case(&format!("{tool}.{extension}")))
}

/// Sanitizes the working directory and project name without returning secret arguments.
fn sanitize_project_context(
    cwd: Option<&Path>,
    argv: &[String],
) -> (Option<String>, Option<String>) {
    let home = std::env::var_os("HOME").map(PathBuf::from);

    // Try cwd first
    if let Some(dir) = cwd {
        let dir_str = dir.to_string_lossy();
        if !dir_str.is_empty() && dir_str != "/" {
            let project_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|n| !n.is_empty() && n != ".");

            let working_dir = if let Some(ref home_path) = home {
                if let Ok(rel) = dir.strip_prefix(home_path) {
                    Some(format!("~/{}", rel.to_string_lossy()))
                } else {
                    Some(dir_str.to_string())
                }
            } else {
                Some(dir_str.to_string())
            };

            return (project_name, working_dir);
        }
    }

    // Fallback: check argv for script paths
    for arg in argv.iter().skip(1) {
        if arg.starts_with('/') {
            let path = Path::new(arg);
            if let Some(parent) = path.parent() {
                let project_name = parent
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .filter(|n| !n.is_empty());
                let working_dir = if let Some(ref home_path) = home {
                    if let Ok(rel) = parent.strip_prefix(home_path) {
                        Some(format!("~/{}", rel.to_string_lossy()))
                    } else {
                        Some(parent.to_string_lossy().to_string())
                    }
                } else {
                    Some(parent.to_string_lossy().to_string())
                };
                return (project_name, working_dir);
            }
        }
    }

    (None, None)
}

fn clean_process_display_name(process_name: &str, raw_command: &str) -> String {
    if !process_name.is_empty() {
        process_name.to_string()
    } else if !raw_command.is_empty() {
        raw_command.to_string()
    } else {
        "Unknown Process".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_vite_development_server() {
        let argv = vec![
            "node".to_string(),
            "/Users/apple/Myproject/clean1/node_modules/vite/bin/vite.js".to_string(),
        ];
        let input = ProcessClassificationInput {
            pid: 32892,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 5173,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/opt/homebrew/bin/node")),
            cwd: Some(Path::new("/Users/apple/Myproject/clean1")),
            argv: &argv,
            started_at: Some(1700000000),
        };

        let result = classify_listener(&input);
        assert!(result.can_release);
        assert_eq!(result.server_name, "Vite");
        assert_eq!(result.project_name.as_deref(), Some("clean1"));
        assert_eq!(result.blocked_reason, None);
    }

    #[test]
    fn classify_nextjs_development_server() {
        let argv = vec![
            "node".to_string(),
            "/Users/apple/work/web-dashboard/node_modules/.bin/next".to_string(),
            "dev".to_string(),
        ];
        let input = ProcessClassificationInput {
            pid: 40001,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 3000,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/usr/local/bin/node")),
            cwd: Some(Path::new("/Users/apple/work/web-dashboard")),
            argv: &argv,
            started_at: Some(1700000000),
        };

        let result = classify_listener(&input);
        assert!(result.can_release);
        assert_eq!(result.server_name, "Next.js");
        assert_eq!(result.project_name.as_deref(), Some("web-dashboard"));
        assert_eq!(result.blocked_reason, None);
    }

    #[test]
    fn classify_python_http_server_and_uvicorn() {
        let argv1 = vec![
            "python3".to_string(),
            "-m".to_string(),
            "http.server".to_string(),
            "8000".to_string(),
        ];
        let input1 = ProcessClassificationInput {
            pid: 50001,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 8000,
            raw_command: "python3",
            process_name: "python3",
            exe_path: Some(Path::new("/usr/bin/python3")),
            cwd: Some(Path::new("/Users/apple/docs")),
            argv: &argv1,
            started_at: Some(1700000000),
        };
        let res1 = classify_listener(&input1);
        assert!(res1.can_release);
        assert_eq!(res1.server_name, "Python http.server");

        let argv2 = vec![
            "uvicorn".to_string(),
            "main:app".to_string(),
            "--reload".to_string(),
        ];
        let input2 = ProcessClassificationInput {
            pid: 50002,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 8080,
            raw_command: "uvicorn",
            process_name: "uvicorn",
            exe_path: Some(Path::new("/opt/homebrew/bin/uvicorn")),
            cwd: Some(Path::new("/Users/apple/api")),
            argv: &argv2,
            started_at: Some(1700000000),
        };
        let res2 = classify_listener(&input2);
        assert!(res2.can_release);
        assert_eq!(res2.server_name, "Uvicorn");
    }

    #[test]
    fn reject_generic_runtime_name_only_cases() {
        let argv = vec!["node".to_string(), "long_running_worker.js".to_string()];
        let input = ProcessClassificationInput {
            pid: 32000,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 5000,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/opt/homebrew/bin/node")),
            cwd: Some(Path::new("/Users/apple/worker")),
            argv: &argv,
            started_at: Some(1700000000),
        };

        let result = classify_listener(&input);
        assert!(!result.can_release);
        assert_eq!(
            result.blocked_reason.as_deref(),
            Some("Not recognized as a development server")
        );
    }

    #[test]
    fn reject_tool_name_substrings_in_unrelated_project_paths() {
        for script in [
            "/Users/apple/invite-service/server.js",
            "/Users/apple/catastrophe-api/server.js",
        ] {
            let argv = vec!["node".to_string(), script.to_string()];
            let input = ProcessClassificationInput {
                pid: 32000,
                uid: Some(501),
                current_user_uid: 501,
                zenith_pid: 1000,
                port: 5000,
                raw_command: "node",
                process_name: "node",
                exe_path: Some(Path::new("/opt/homebrew/bin/node")),
                cwd: Some(Path::new("/Users/apple/project")),
                argv: &argv,
                started_at: Some(1700000000),
            };

            assert!(!classify_listener(&input).can_release, "script: {script}");
        }
    }

    #[test]
    fn reject_listener_without_stable_process_identity() {
        let argv = vec!["node".to_string(), "vite.js".to_string()];
        for (uid, started_at, exe_path) in [
            (
                None,
                Some(1700000000),
                Some(Path::new("/opt/homebrew/bin/node")),
            ),
            (Some(501), None, Some(Path::new("/opt/homebrew/bin/node"))),
            (Some(501), Some(1700000000), None),
        ] {
            let input = ProcessClassificationInput {
                pid: 32892,
                uid,
                current_user_uid: 501,
                zenith_pid: 1000,
                port: 5173,
                raw_command: "node",
                process_name: "node",
                exe_path,
                cwd: Some(Path::new("/Users/apple/project")),
                argv: &argv,
                started_at,
            };

            let result = classify_listener(&input);
            assert!(!result.can_release);
            assert_eq!(
                result.blocked_reason.as_deref(),
                Some("Process identity is unavailable")
            );
        }
    }

    #[test]
    fn reject_root_system_and_other_user_processes() {
        let input_root = ProcessClassificationInput {
            pid: 1234,
            uid: Some(0),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 8080,
            raw_command: "nginx",
            process_name: "nginx",
            exe_path: Some(Path::new("/usr/sbin/nginx")),
            cwd: None,
            argv: &[],
            started_at: Some(1700000000),
        };
        let res_root = classify_listener(&input_root);
        assert!(!res_root.can_release);
        assert_eq!(
            res_root.blocked_reason.as_deref(),
            Some("Root-owned system process")
        );

        let input_other = ProcessClassificationInput {
            pid: 2345,
            uid: Some(502),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 8080,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/opt/homebrew/bin/node")),
            cwd: None,
            argv: &[],
            started_at: Some(1700000000),
        };
        let res_other = classify_listener(&input_other);
        assert!(!res_other.can_release);
        assert_eq!(
            res_other.blocked_reason.as_deref(),
            Some("Owned by another user")
        );
    }

    #[test]
    fn reject_privileged_ports_below_1024() {
        let argv = vec!["node".to_string(), "vite.js".to_string()];
        let input = ProcessClassificationInput {
            pid: 3000,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 80,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/opt/homebrew/bin/node")),
            cwd: None,
            argv: &argv,
            started_at: Some(1700000000),
        };
        let res = classify_listener(&input);
        assert!(!res.can_release);
        assert_eq!(
            res.blocked_reason.as_deref(),
            Some("Privileged system port (below 1024)")
        );
    }

    #[test]
    fn reject_protected_system_terminal_database_and_zenith_processes() {
        // PostgreSQL
        let input_pg = ProcessClassificationInput {
            pid: 5432,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 5432,
            raw_command: "postgres",
            process_name: "postgres",
            exe_path: Some(Path::new("/opt/homebrew/bin/postgres")),
            cwd: None,
            argv: &[],
            started_at: Some(1700000000),
        };
        let res_pg = classify_listener(&input_pg);
        assert!(!res_pg.can_release);
        assert_eq!(
            res_pg.blocked_reason.as_deref(),
            Some("Protected system, terminal, database, or container process")
        );

        // Terminal / SSH / Dockerd
        let input_ssh = ProcessClassificationInput {
            pid: 2222,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 2222,
            raw_command: "sshd",
            process_name: "sshd",
            exe_path: Some(Path::new("/usr/sbin/sshd")),
            cwd: None,
            argv: &[],
            started_at: Some(1700000000),
        };
        assert!(!classify_listener(&input_ssh).can_release);

        // Zenith itself
        let input_zenith = ProcessClassificationInput {
            pid: 1000,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 9000,
            raw_command: "Zenith",
            process_name: "Zenith",
            exe_path: Some(Path::new("/Applications/Zenith.app/Contents/MacOS/Zenith")),
            cwd: None,
            argv: &[],
            started_at: Some(1700000000),
        };
        let res_zenith = classify_listener(&input_zenith);
        assert!(!res_zenith.can_release);
        assert_eq!(res_zenith.server_name, "Zenith");
    }

    #[test]
    fn sanitize_labels_and_project_context_without_returning_argv_secrets() {
        let argv = vec![
            "node".to_string(),
            "/Users/apple/secret-project/node_modules/vite/bin/vite.js".to_string(),
            "--token=SUPER_SECRET_TOKEN_12345".to_string(),
            "--key=SECRET_API_KEY".to_string(),
        ];
        let input = ProcessClassificationInput {
            pid: 32892,
            uid: Some(501),
            current_user_uid: 501,
            zenith_pid: 1000,
            port: 5173,
            raw_command: "node",
            process_name: "node",
            exe_path: Some(Path::new("/opt/homebrew/bin/node")),
            cwd: Some(Path::new("/Users/apple/secret-project")),
            argv: &argv,
            started_at: Some(1700000000),
        };

        let result = classify_listener(&input);
        assert!(result.can_release);
        assert_eq!(result.server_name, "Vite");
        assert_eq!(result.project_name.as_deref(), Some("secret-project"));
        assert!(!result.server_name.contains("SUPER_SECRET"));
        assert!(!result.server_name.contains("SECRET_API_KEY"));
        if let Some(wd) = result.working_directory {
            assert!(!wd.contains("SUPER_SECRET"));
        }
    }
}
