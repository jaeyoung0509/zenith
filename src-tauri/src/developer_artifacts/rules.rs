//! One catalog owns recognition, descent boundaries, and allowed relative paths.
//! Order is significant when ecosystems share an artifact directory.
use super::*;

struct Rule {
    discovery_name: Option<&'static str>,
    relative: &'static str,
    kind: DeveloperArtifactKind,
    recognition: Recognition,
}

enum Recognition {
    Marker {
        names: &'static [&'static str],
        extensions: &'static [&'static str],
        ecosystem: DeveloperEcosystem,
        hint: &'static str,
    },
    Custom(fn(&Path, &str) -> Option<ArtifactMatch>),
    // Go's shared cache is discovered only by the separately scoped home adapter.
    GlobalGo,
}

const RULES: &[Rule] = &[
    Rule {
        discovery_name: Some("target"),
        relative: "target",
        kind: DeveloperArtifactKind::CargoTarget,
        recognition: Recognition::Marker {
            names: &["Cargo.toml"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Rust,
            hint: "cargo build",
        },
    },
    Rule {
        discovery_name: Some("target"),
        relative: "target",
        kind: DeveloperArtifactKind::MavenTarget,
        recognition: Recognition::Marker {
            names: &["pom.xml"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Java,
            hint: "mvn clean package",
        },
    },
    Rule {
        discovery_name: Some("target"),
        relative: "target",
        kind: DeveloperArtifactKind::SbtTarget,
        recognition: Recognition::Marker {
            names: &["build.sbt"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Scala,
            hint: "sbt compile",
        },
    },
    Rule {
        discovery_name: Some("target"),
        relative: "target",
        kind: DeveloperArtifactKind::ClojureTarget,
        recognition: Recognition::Marker {
            names: &["project.clj", "deps.edn"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Clojure,
            hint: "clojure -T:build compile",
        },
    },
    Rule {
        discovery_name: Some("node_modules"),
        relative: "node_modules",
        kind: DeveloperArtifactKind::NodeModules,
        recognition: Recognition::Custom(|root, _| recognize_node(root)),
    },
    Rule {
        discovery_name: Some(".venv"),
        relative: ".venv",
        kind: DeveloperArtifactKind::PythonVenv,
        recognition: Recognition::Custom(recognize_python),
    },
    Rule {
        discovery_name: Some("venv"),
        relative: "venv",
        kind: DeveloperArtifactKind::PythonVenv,
        recognition: Recognition::Custom(recognize_python),
    },
    Rule {
        discovery_name: Some("vendor"),
        relative: "vendor",
        kind: DeveloperArtifactKind::ComposerVendor,
        recognition: Recognition::Custom(|root, _| recognize_vendor(root)),
    },
    Rule {
        discovery_name: Some("vendor"),
        relative: "vendor/bundle",
        kind: DeveloperArtifactKind::RubyBundle,
        recognition: Recognition::Custom(|root, _| recognize_vendor(root)),
    },
    Rule {
        discovery_name: Some("build"),
        relative: "build",
        kind: DeveloperArtifactKind::GradleBuild,
        recognition: Recognition::Custom(recognize_build),
    },
    Rule {
        discovery_name: Some("build"),
        relative: "build",
        kind: DeveloperArtifactKind::CMakeBuild,
        recognition: Recognition::Custom(recognize_build),
    },
    Rule {
        discovery_name: Some(".gradle"),
        relative: ".gradle",
        kind: DeveloperArtifactKind::GradleCache,
        recognition: Recognition::Marker {
            names: &["build.gradle.kts"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Kotlin,
            hint: "./gradlew build",
        },
    },
    Rule {
        discovery_name: Some(".gradle"),
        relative: ".gradle",
        kind: DeveloperArtifactKind::GradleCache,
        recognition: Recognition::Marker {
            names: &["build.gradle"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Java,
            hint: "./gradlew build",
        },
    },
    Rule {
        discovery_name: Some(".gradle"),
        relative: ".gradle",
        kind: DeveloperArtifactKind::GradleCache,
        recognition: Recognition::Marker {
            names: &["settings.gradle.kts"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Kotlin,
            hint: "./gradlew build",
        },
    },
    Rule {
        discovery_name: Some(".gradle"),
        relative: ".gradle",
        kind: DeveloperArtifactKind::GradleCache,
        recognition: Recognition::Marker {
            names: &["settings.gradle"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Java,
            hint: "./gradlew build",
        },
    },
    Rule {
        discovery_name: Some(".gradle"),
        relative: ".gradle",
        kind: DeveloperArtifactKind::GradleCache,
        recognition: Recognition::Marker {
            names: &["gradlew"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Java,
            hint: "./gradlew build",
        },
    },
    Rule {
        discovery_name: Some("bin"),
        relative: "bin",
        kind: DeveloperArtifactKind::DotnetBin,
        recognition: Recognition::Marker {
            names: &[],
            extensions: &["csproj", "fsproj", "vbproj", "sln"],
            ecosystem: DeveloperEcosystem::Dotnet,
            hint: "dotnet restore",
        },
    },
    Rule {
        discovery_name: Some("obj"),
        relative: "obj",
        kind: DeveloperArtifactKind::DotnetObj,
        recognition: Recognition::Marker {
            names: &[],
            extensions: &["csproj", "fsproj", "vbproj", "sln"],
            ecosystem: DeveloperEcosystem::Dotnet,
            hint: "dotnet restore",
        },
    },
    Rule {
        discovery_name: Some(".build"),
        relative: ".build",
        kind: DeveloperArtifactKind::SwiftBuild,
        recognition: Recognition::Marker {
            names: &["Package.swift"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Swift,
            hint: "swift build",
        },
    },
    Rule {
        discovery_name: Some(".dart_tool"),
        relative: ".dart_tool",
        kind: DeveloperArtifactKind::FlutterTooling,
        recognition: Recognition::Marker {
            names: &["pubspec.yaml"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Dart,
            hint: "flutter pub get",
        },
    },
    Rule {
        discovery_name: Some("_build"),
        relative: "_build",
        kind: DeveloperArtifactKind::ElixirBuild,
        recognition: Recognition::Marker {
            names: &["mix.exs"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Elixir,
            hint: "mix deps.get",
        },
    },
    Rule {
        discovery_name: Some("_build"),
        relative: "_build",
        kind: DeveloperArtifactKind::ErlangBuild,
        recognition: Recognition::Marker {
            names: &["rebar.config"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Erlang,
            hint: "rebar3 compile",
        },
    },
    Rule {
        discovery_name: Some("deps"),
        relative: "deps",
        kind: DeveloperArtifactKind::ElixirDeps,
        recognition: Recognition::Marker {
            names: &["mix.exs"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Elixir,
            hint: "mix deps.get",
        },
    },
    Rule {
        discovery_name: Some(".stack-work"),
        relative: ".stack-work",
        kind: DeveloperArtifactKind::HaskellStackWork,
        recognition: Recognition::Marker {
            names: &["stack.yaml", "cabal.project"],
            extensions: &["cabal"],
            ecosystem: DeveloperEcosystem::Haskell,
            hint: "stack build",
        },
    },
    Rule {
        discovery_name: Some("dist-newstyle"),
        relative: "dist-newstyle",
        kind: DeveloperArtifactKind::HaskellDistNewstyle,
        recognition: Recognition::Marker {
            names: &["stack.yaml", "cabal.project"],
            extensions: &["cabal"],
            ecosystem: DeveloperEcosystem::Haskell,
            hint: "cabal build",
        },
    },
    Rule {
        discovery_name: Some(".zig-cache"),
        relative: ".zig-cache",
        kind: DeveloperArtifactKind::ZigCache,
        recognition: Recognition::Marker {
            names: &["build.zig"],
            extensions: &[],
            ecosystem: DeveloperEcosystem::Zig,
            hint: "zig build",
        },
    },
    Rule {
        discovery_name: Some(".terraform"),
        relative: ".terraform",
        kind: DeveloperArtifactKind::TerraformCache,
        recognition: Recognition::Marker {
            names: &[".terraform.lock.hcl"],
            extensions: &["tf"],
            ecosystem: DeveloperEcosystem::Terraform,
            hint: "terraform init",
        },
    },
    Rule {
        discovery_name: None,
        relative: "pkg/mod",
        kind: DeveloperArtifactKind::GoModuleCache,
        recognition: Recognition::GlobalGo,
    },
];

pub(super) fn recognize(root: &Path, name: &str) -> Option<ArtifactMatch> {
    RULES
        .iter()
        .filter(|rule| rule.discovery_name == Some(name))
        .find_map(|rule| {
            let found = match &rule.recognition {
                Recognition::Marker {
                    names,
                    extensions,
                    ecosystem,
                    hint,
                } => {
                    let marker = find_named_marker(root, names)
                        .or_else(|| find_project_extension_marker(root, extensions))?;
                    ArtifactMatch {
                        ecosystem: *ecosystem,
                        kind: rule.kind,
                        project_root: root.to_path_buf(),
                        artifact_relative: PathBuf::from(rule.relative),
                        evidence: vec![marker.file_name()?.to_string_lossy().into_owned()],
                        marker_paths: vec![marker],
                        rebuild_hint: Some((*hint).to_string()),
                    }
                }
                Recognition::Custom(recognize) => recognize(root, name)?,
                Recognition::GlobalGo => return None,
            };
            // Custom evidence cannot authorize a path or kind absent from its row.
            (found.kind == rule.kind && found.artifact_relative == Path::new(rule.relative))
                .then_some(found)
        })
}

pub(super) fn is_artifact_directory(name: &str) -> bool {
    RULES.iter().any(|rule| rule.discovery_name == Some(name))
}

pub(crate) fn artifact_relative_is_allowed(relative: &Path, kind: DeveloperArtifactKind) -> bool {
    RULES
        .iter()
        .any(|rule| rule.kind == kind && relative == Path::new(rule.relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_rules_require_direct_evidence_and_authorize_only_their_relative_path() {
        for (rule, names, extensions, ecosystem, hint) in
            RULES.iter().filter_map(|rule| match &rule.recognition {
                Recognition::Marker {
                    names,
                    extensions,
                    ecosystem,
                    hint,
                } => Some((rule, names, extensions, ecosystem, hint)),
                _ => None,
            })
        {
            for marker_name in names.iter().map(|name| (*name).to_string()).chain(
                extensions
                    .iter()
                    .map(|extension| format!("project.{extension}")),
            ) {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().join("project");
                let sibling = temp.path().join("sibling");
                fs::create_dir_all(root.join(rule.relative)).unwrap();
                fs::create_dir_all(&sibling).unwrap();
                fs::write(temp.path().join(&marker_name), "ancestor").unwrap();
                fs::write(sibling.join(&marker_name), "sibling").unwrap();
                let name = rule.discovery_name.unwrap();
                assert!(recognize(&root, name).is_none(), "{name}: {marker_name}");
                fs::write(root.join(&marker_name), "direct").unwrap();
                let found = recognize(&root, name).expect("direct marker authorizes rule");
                assert_eq!(found.kind, rule.kind);
                assert_eq!(found.ecosystem, *ecosystem);
                assert_eq!(found.rebuild_hint.as_deref(), Some(*hint));
                assert_eq!(found.marker_paths, vec![root.join(&marker_name)]);
                assert!(is_artifact_directory(name));
                assert!(artifact_relative_is_allowed(
                    &found.artifact_relative,
                    found.kind
                ));
                assert!(!artifact_relative_is_allowed(
                    &PathBuf::from("other").join(rule.relative),
                    found.kind
                ));
                assert!(!artifact_relative_is_allowed(
                    &PathBuf::from(rule.relative).join("child"),
                    found.kind
                ));
            }
        }
    }

    #[test]
    fn shared_go_cache_is_not_a_generic_project_or_descent_rule() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("pkg/mod")).unwrap();
        fs::write(temp.path().join("go.mod"), "module fixture").unwrap();
        assert!(recognize(temp.path(), "pkg").is_none());
        assert!(!is_artifact_directory("pkg"));
        assert!(artifact_relative_is_allowed(
            Path::new("pkg/mod"),
            DeveloperArtifactKind::GoModuleCache
        ));
        assert!(!artifact_relative_is_allowed(
            Path::new("pkg"),
            DeveloperArtifactKind::GoModuleCache
        ));
        assert!(!artifact_relative_is_allowed(
            Path::new("target"),
            DeveloperArtifactKind::NodeModules
        ));
    }
}
