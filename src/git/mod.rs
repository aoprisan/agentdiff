//! The only place libgit2 / the filesystem walk lives. Read-only: builds a
//! `domain::Diff`, never mutates the working tree.

pub mod differ;
pub mod repo;
pub mod untracked;

pub use differ::diff_worktree_vs_head;
pub use repo::Repo;
