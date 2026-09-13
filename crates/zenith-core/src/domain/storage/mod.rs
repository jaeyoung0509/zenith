//! Storage-management policy: the vocabulary a storage workflow presents and
//! the thresholds that decide what it is willing to show.
//!
//! These enums are not decoration around a wire field. [`LargeFileFilter`]
//! owns the byte threshold a filter implies and the extensions an installer
//! scan accepts, so the scanner and the command layer agree on what "large
//! file" and "installer" mean by construction rather than by both reading the
//! same literal.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LargeFileKind {
    Video,
    Archive,
    DiskImage,
    Installer,
    VmImage,
    AiModel,
    Database,
    DeveloperArtifact,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LargeFileFilter {
    #[default]
    All,
    Installers,
}

impl LargeFileFilter {
    /// Smallest size this filter will report.
    ///
    /// An installer is usually a few hundred megabytes while a video is
    /// usually several gigabytes, so the two filters cannot share a floor
    /// without hiding installers or drowning them.
    pub fn minimum_threshold(self) -> u64 {
        match self {
            Self::All => 100 * 1024 * 1024,
            Self::Installers => 10 * 1024 * 1024,
        }
    }

    pub fn matches_extension(self, extension: Option<&str>) -> bool {
        match self {
            Self::All => true,
            Self::Installers => matches!(extension, Some("dmg" | "pkg" | "mpkg" | "xip" | "iso")),
        }
    }
}

/// How an installed application was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppInstallSource {
    ApplicationBundle,
    HomebrewCask,
    InstallerPackage,
    Unknown,
}

/// How strongly a path is tied to the application being uninstalled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppRelatedConfidence {
    High,
    Medium,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AppRelatedKind {
    AppBundle,
    ApplicationSupport,
    Cache,
    Log,
    Preference,
    SavedState,
    Container,
    GroupContainer,
    ApplicationScripts,
    HttpStorage,
    WebKit,
}
