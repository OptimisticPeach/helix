use anyhow::{bail, Context, Result};
use arc_swap::ArcSwap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use subversion::client::CatOptions;

use subversion as svn;

use crate::FileChange;

pub fn get_diff_base(file: &Path) -> Result<Vec<u8>> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());

    let mut client = match svn::client::Context::new() {
        Ok(x) => x,
        Err(e) => bail!("{e}"),
    };

    let mut buffer = Vec::new();

    let options = CatOptions {
        revision: subversion::Revision::Base,
        peg_revision: subversion::Revision::Unspecified,
        expand_keywords: false,
    };

    if let Err(e) = client.cat(
        file.to_str()
            .context("Couldn't convert path to UTF8 string")?,
        &mut buffer,
        &options,
    ) {
        bail!("{e}");
    }

    Ok(buffer)
}

pub fn get_current_head_name(file: &Path) -> Result<Arc<ArcSwap<Box<str>>>> {
    debug_assert!(!file.exists() || file.is_file());
    debug_assert!(file.is_absolute());

    let file = file.to_str().context("Could not convert path to UTF8")?;

    let mut rev = (svn::Revnum::invalid(), svn::Revnum::invalid());

    match svn::client::Context::new().and_then(|mut v| {
        rev = v.min_max_revisions(file, true)?;

        Ok(())
    }) {
        Ok(x) => x,
        Err(e) => {
            log::error!("Subversion Error: {e}");
            bail!("{e}")
        }
    };

    let (_min, max) = rev;

    if max == svn::Revnum::invalid() {
        bail!("Couldn't fetch svn revision number");
    }

    Ok(Arc::new(ArcSwap::from_pointee(
        format!("R:{}", max.as_i64()).into_boxed_str(),
    )))
}

/// Emulates the result of running `svn status` from the command line.
pub fn for_each_changed_file(cwd: &Path, f: impl Fn(Result<FileChange>) -> bool) -> Result<()> {
    let mut client = match svn::client::Context::new() {
        Ok(x) => x,
        Err(e) => bail!("{e}"),
    };

    let result = client.status(
        cwd.to_str()
            .context("Path could not be converted to UTF8")?,
        &svn::client::StatusOptions {
            revision: svn::Revision::Working,
            depth: svn::Depth::Infinity,
            get_all: false,
            check_out_of_date: false,
            check_working_copy: true,
            no_ignore: false,
            ignore_externals: false,
            depth_as_sticky: false,
            changelists: None,
        },
        &|path, status| {
            let path = PathBuf::from(path);

            let file_status = match status.node_status() {
                subversion::StatusKind::None => return Ok(()),
                subversion::StatusKind::Unversioned => FileChange::Untracked { path },
                subversion::StatusKind::Normal => return Ok(()),
                subversion::StatusKind::Added => FileChange::Untracked { path },
                subversion::StatusKind::Missing => FileChange::Deleted { path },
                subversion::StatusKind::Deleted => FileChange::Deleted { path },
                subversion::StatusKind::Replaced => FileChange::Renamed {
                    from_path: status.moved_from_abspath().unwrap().into(),
                    to_path: status.moved_to_abspath().unwrap().into(),
                },
                subversion::StatusKind::Modified => FileChange::Modified { path },
                subversion::StatusKind::Merged => FileChange::Modified { path },
                subversion::StatusKind::Conflicted => FileChange::Conflict { path },
                subversion::StatusKind::Ignored => FileChange::Untracked { path },
                subversion::StatusKind::Obstructed => FileChange::Untracked { path },
                subversion::StatusKind::External => FileChange::Untracked { path },
                subversion::StatusKind::Incomplete => FileChange::Untracked { path },
            };

            if !f(Ok(file_status)) {
                return Err(svn::error::Error::from_message("break"));
            }

            Ok(())
        },
    );

    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            if e.message() == Some("break") {
                Ok(())
            } else {
                bail!("{e}")
            }
        }
    }
}
