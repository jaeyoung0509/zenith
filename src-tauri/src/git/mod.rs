//! Reading Git state from a directory Zenith did not create.
//!
//! A project root comes from an observed agent process working directory, so
//! the repositories Zenith reads are the directories the user happens to work
//! in. Zenith reads state there; it does not execute or rewrite the
//! configuration it finds. This module owns both halves of that boundary:
//!
//! - [`GitInspection`] and the single invocation constructor ([`git_command`])
//!   decide whether `git` may run in a root, and build every command that does;
//! - [`git_directory`] decides which git directory describes a root, and
//!   refuses a `.git` pointer that is not tied to this checkout;
//! - `read_capped` bounds every metadata file a pointer makes Zenith read.
//!
//! The boundary distinguishes two kinds of program-naming configuration. The
//! fixed keys Git reads for every operation (`core.fsmonitor`, `core.hooksPath`,
//! `core.pager`, `core.sshCommand`, `diff.external`) are replaced on the command
//! line, where Git gives them precedence. The driver definitions
//! (`filter.<driver>.*`, `diff.<driver>.*`, `merge.<driver>.*`) cannot be
//! replaced key by key — a repository can invent any driver name — so a
//! repository whose own configuration defines one at inspection or command
//! construction time is refused instead of read, and the reason is reported.
//! Everything else, including the machine's Git
//! configuration and the repository's attribute files, is honored: a clean
//! checkout has to read as clean to the user, and neutralizing those made
//! Zenith's answer differ from the user's own `git status`.

mod command;
pub(crate) mod metadata;
mod repository;

pub use command::{git_command, GitInspection, GitRefusal};
pub(crate) use metadata::{read_capped, CappedRead};
pub use repository::{git_directory, GitDirectory};
