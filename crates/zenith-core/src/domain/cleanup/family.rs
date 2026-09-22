//! Cleanup catalog ownership.
//!
//! A category is how the interface groups observations. A cleaner family is
//! who owns the knowledge required to act on one: a package-manager store and
//! an Xcode build cache can both appear under Developer while requiring very
//! different discovery and execution contracts.

use serde::{Deserialize, Serialize};

use crate::domain::category::Category;

/// The module that owns a cleanup catalog entry's discovery and recovery
/// contract.
///
/// `Unclassified` exists only as the serde/default sentinel for older fixtures.
/// A validated catalog never contains it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "snake_case")]
pub enum CleanerFamily {
    #[default]
    Unclassified,
    User,
    System,
    Applications,
    Developer,
    PackageManagers,
    Containers,
    Leftovers,
}

impl CleanerFamily {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Unclassified => "Unclassified",
            Self::User => "User data",
            Self::System => "System",
            Self::Applications => "Applications",
            Self::Developer => "Developer tools",
            Self::PackageManagers => "Package managers",
            Self::Containers => "Containers",
            Self::Leftovers => "Application leftovers",
        }
    }

    /// Whether this owner family may publish observations in an interface
    /// category.
    ///
    /// This is intentionally broader than a one-to-one mapping. For example,
    /// application caches produced by an AI editor appear in the AI category,
    /// while an Xcode device-support cleaner appears under Developer even when
    /// its path is declared in the system catalog file.
    pub fn accepts_category(self, category: Category) -> bool {
        match self {
            Self::Unclassified => false,
            Self::User | Self::System | Self::Leftovers => category == Category::System,
            Self::Applications => matches!(category, Category::Ai | Category::System),
            Self::Developer => {
                matches!(
                    category,
                    Category::Ai | Category::Developer | Category::System
                )
            }
            Self::PackageManagers => category == Category::Developer,
            Self::Containers => category == Category::Container,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CleanerFamily;
    use crate::domain::category::Category;

    #[test]
    fn families_state_the_categories_they_can_own() {
        assert!(CleanerFamily::PackageManagers.accepts_category(Category::Developer));
        assert!(!CleanerFamily::PackageManagers.accepts_category(Category::System));
        assert!(CleanerFamily::Applications.accepts_category(Category::Ai));
        assert!(CleanerFamily::Developer.accepts_category(Category::System));
        assert!(CleanerFamily::Containers.accepts_category(Category::Container));
        assert!(!CleanerFamily::Unclassified.accepts_category(Category::System));
    }
}
