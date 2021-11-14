//! Handling of nix GC roots
//!
//! TODO: inline this module into `::project`
use crate::builder::{OutputPath, RootedPath};
use crate::project::Project;
use crate::AbsPathBuf;
use slog::debug;
use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Roots manipulation
#[derive(Clone)]
pub struct Roots {
    /// The GC root directory in the lorri user cache dir
    gc_root_path: AbsPathBuf,
    /// Unique ID generated from a project’s path, which can be used as a directory name.
    project_id: String,
}

/// A path to a gc root.
#[derive(Hash, PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
pub struct RootPath(pub AbsPathBuf);

impl RootPath {
    /// `display` the path.
    pub fn display(&self) -> std::path::Display {
        self.0.display()
    }
}

impl OutputPath<RootPath> {
    /// Check whether all all GC roots exist.
    pub fn all_exist(&self) -> bool {
        let crate::builder::OutputPath { shell_gc_root } = self;

        shell_gc_root.0.as_absolute_path().exists()
    }
}

impl Roots {
    // TODO: all use-cases are from_project; just save a reference to a project?
    /// Construct a Roots struct based on a project's GC root directory
    /// and ID.
    pub fn from_project(project: &Project) -> Roots {
        Roots {
            gc_root_path: project.gc_root_path.clone(),
            project_id: project.hash().to_string(),
        }
    }

    // final path in the `self.gc_root_path` directory,
    // the symlink which points to the lorri-keep-env-hack-nix-shell drv (see ./logged-evaluation.nix)
    fn shell_gc_root(&self) -> AbsPathBuf {
        self.gc_root_path.join("shell_gc_root")
    }

    /// Return the filesystem paths for these roots.
    pub fn paths(&self) -> OutputPath<RootPath> {
        OutputPath {
            shell_gc_root: RootPath(self.shell_gc_root()),
        }
    }

    /// Create roots to store paths.
    pub fn create_roots(
        &self,
        // Important: this intentionally only allows creating
        // roots to `StorePath`, not to `DrvFile`, because we have
        // no use case for creating GC roots for drv files.
        path: RootedPath,
        logger: &slog::Logger,
    ) -> Result<OutputPath<RootPath>, AddRootError>
where {
        let store_path = &path.path;

        debug!(logger, "adding root"; "from" => store_path.as_path().to_str(), "to" => self.shell_gc_root().display());
        std::fs::remove_file(&self.shell_gc_root())
            .or_else(|e| AddRootError::remove(e, &self.shell_gc_root().as_absolute_path()))?;

        // the forward GC root that points from the store path to our cache gc_roots dir
        std::os::unix::fs::symlink(store_path.as_path(), &self.shell_gc_root()).map_err(|e| {
            AddRootError::symlink(
                e,
                store_path.as_path(),
                self.shell_gc_root().as_absolute_path(),
            )
        })?;

        // the reverse GC root that points from nix to our cache gc_roots dir
        let mut root = if let Ok(path) = env::var("NIX_STATE_DIR") {
            PathBuf::from(path)
        } else {
            PathBuf::from("/nix/var/nix/")
        };
        root.push("gcroots");
        root.push("per-user");

        // TODO: check on start of lorri
        root.push(env::var("USER").expect("env var 'USER' must be set"));

        // The user directory sometimes doesn’t exist,
        // but we can create it (it’s root but `rwxrwxrwx`)
        if !root.is_dir() {
            std::fs::create_dir_all(&root).map_err(|source| AddRootError {
                source,
                msg: format!("Failed to recursively create directory {}", root.display()),
            })?
        }

        // We register a garbage collection root, which points back to our `~/.cache/lorri/gc_roots` directory,
        // so that nix won’t delete our shell environment.
        root.push(format!("{}-{}", self.project_id, "shell_gc_root"));

        debug!(logger, "connecting root"; "from" => self.shell_gc_root().display(), "to" => root.to_str());
        std::fs::remove_file(&root).or_else(|e| AddRootError::remove(e, &root))?;

        std::os::unix::fs::symlink(&self.shell_gc_root(), &root).map_err(|e| {
            AddRootError::symlink(e, self.shell_gc_root().as_absolute_path(), &root)
        })?;

        // TODO: don’t return the RootPath here
        Ok(OutputPath {
            shell_gc_root: RootPath(self.shell_gc_root()),
        })
    }
}

/// Error conditions encountered when adding roots
#[derive(Error, Debug)]
#[error("{msg}: {source}")]
pub struct AddRootError {
    #[source]
    source: std::io::Error,
    msg: String,
}

impl AddRootError {
    /// Ignore NotFound errors (it is after all a remove), and otherwise
    /// return an error explaining a delete on path failed.
    fn remove(source: std::io::Error, path: &Path) -> Result<(), AddRootError> {
        if source.kind() == std::io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(AddRootError {
                source,
                msg: format!("Failed to delete {}", path.display()),
            })
        }
    }

    /// Return an error explaining what symlink failed
    fn symlink(source: std::io::Error, src: &Path, dest: &Path) -> AddRootError {
        AddRootError {
            source,
            msg: format!("Failed to symlink {} to {}", src.display(), dest.display()),
        }
    }
}
