//! Shared bounded filesystem-tree validation, including implicit ZIP parents.

use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

use crate::error::LambdaServiceError;

const MAX_NODES: usize = 10_000;

#[derive(Debug, Default)]
pub(crate) struct ZipLayout {
    nodes: BTreeMap<PathBuf, bool>,
    explicit: HashSet<PathBuf>,
}

impl ZipLayout {
    /// Validate a member and return newly required directories, root first.
    pub(crate) fn admit(
        &mut self,
        path: &Path,
        directory: bool,
    ) -> Result<Vec<PathBuf>, LambdaServiceError> {
        if path.file_name().is_none() || !self.explicit.insert(path.to_owned()) {
            return Err(invalid());
        }
        let mut parents: Vec<&Path> = path
            .ancestors()
            .skip(1)
            .filter(|parent| parent.file_name().is_some())
            .collect();
        parents.reverse();
        let mut created = Vec::new();
        for parent in parents {
            match self.nodes.get(parent) {
                Some(false) => return Err(invalid()),
                Some(true) => {}
                None => {
                    self.insert(parent, true)?;
                    created.push(parent.to_owned());
                }
            }
        }
        match self.nodes.get(path) {
            Some(true) if directory => {}
            Some(_) => return Err(invalid()),
            None => {
                self.insert(path, directory)?;
                if directory {
                    created.push(path.to_owned());
                }
            }
        }
        Ok(created)
    }
    fn insert(&mut self, path: &Path, directory: bool) -> Result<(), LambdaServiceError> {
        if self.nodes.len() >= MAX_NODES {
            return Err(invalid());
        }
        self.nodes.insert(path.to_owned(), directory);
        Ok(())
    }
}

fn invalid() -> LambdaServiceError {
    LambdaServiceError::InvalidZipFile {
        message: "Invalid or oversized ZIP filesystem tree".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_bound_the_complete_tree_including_implicit_directories() {
        let mut layout = ZipLayout::default();
        assert_eq!(
            layout.admit(Path::new("root/branch/file"), false).unwrap(),
            vec![PathBuf::from("root"), PathBuf::from("root/branch")]
        );
        assert_eq!(
            layout.admit(Path::new("root/branch"), true).unwrap(),
            [] as [PathBuf; 0]
        );
        for n in 3..MAX_NODES {
            layout.admit(Path::new(&format!("f{n}")), false).unwrap();
        }
        assert_eq!(layout.nodes.len(), MAX_NODES);
        assert!(layout.admit(Path::new("one-more"), false).is_err());
    }

    #[test]
    fn test_should_reject_duplicate_and_parent_file_conflicts_in_either_order() {
        for reverse in [false, true] {
            let mut layout = ZipLayout::default();
            let (first, second) = if reverse {
                ("parent/file", "parent")
            } else {
                ("parent", "parent/file")
            };
            layout.admit(Path::new(first), false).unwrap();
            assert!(layout.admit(Path::new(second), false).is_err());
        }
        let mut layout = ZipLayout::default();
        layout.admit(Path::new("directory"), true).unwrap();
        assert!(layout.admit(Path::new("directory"), true).is_err());
    }
}
