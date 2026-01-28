//! Adapter module
//!
//! Defines the RunnerAdapter trait and concrete implementations.

mod traits;
pub mod node;
pub mod xcode;

pub use traits::*;

use std::path::Path;

/// Registry of all available adapters
pub struct AdapterRegistry {
    // Adapters are registered at compile time
}

impl AdapterRegistry {
    /// Detect project(s) at path
    ///
    /// - If path itself is a project, returns just that project
    /// - If path contains subprojects, returns all found (up to 3 levels deep)
    pub fn detect(path: &Path) -> Vec<Box<dyn RunnerAdapter>> {
        // First: check if path itself is a project
        let mut direct: Vec<Box<dyn RunnerAdapter>> = Vec::new();
        if let Some(adapter) = xcode::XcodeAdapter::detect(path) {
            direct.push(adapter);
        }
        if let Some(adapter) = node::NodeAdapter::detect(path) {
            direct.push(adapter);
        }

        // If we found project(s) at the root, return them
        if !direct.is_empty() {
            return direct;
        }

        // Otherwise, search subdirectories (up to 3 levels)
        Self::scan_with_depth(path, 3)
    }

    /// Scan with limited depth
    fn scan_with_depth(path: &Path, max_depth: usize) -> Vec<Box<dyn RunnerAdapter>> {
        let mut adapters: Vec<Box<dyn RunnerAdapter>> = Vec::new();

        // Check current directory
        adapters.extend(xcode::XcodeAdapter::detect_all(path));
        adapters.extend(node::NodeAdapter::detect_all(path));

        // Recurse if depth allows
        if max_depth > 0 {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let entry_path = entry.path();
                    if entry_path.is_dir() {
                        let name = entry_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if !name.starts_with('.')
                            && name != "node_modules"
                            && name != "target"
                            && name != "build"
                            && name != "DerivedData"
                            && name != "Pods"
                        {
                            adapters.extend(Self::scan_with_depth(&entry_path, max_depth - 1));
                        }
                    }
                }
            }
        }

        adapters
    }

    /// Detect all projects in a directory (non-recursive)
    pub fn detect_all(path: &Path) -> Vec<Box<dyn RunnerAdapter>> {
        let mut adapters: Vec<Box<dyn RunnerAdapter>> = Vec::new();

        // Collect all detected projects
        adapters.extend(xcode::XcodeAdapter::detect_all(path));
        adapters.extend(node::NodeAdapter::detect_all(path));

        adapters
    }

    /// Scan directory recursively for all projects
    pub fn scan(path: &Path) -> Vec<Box<dyn RunnerAdapter>> {
        let mut adapters: Vec<Box<dyn RunnerAdapter>> = Vec::new();

        // First check current directory
        adapters.extend(Self::detect_all(path));

        // Then scan subdirectories
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    // Skip hidden directories and common non-project directories
                    let name = entry_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !name.starts_with('.')
                        && name != "node_modules"
                        && name != "target"
                        && name != "build"
                        && name != "DerivedData"
                    {
                        adapters.extend(Self::scan(&entry_path));
                    }
                }
            }
        }

        adapters
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_empty_dir() {
        let dir = tempdir().unwrap();
        let result = AdapterRegistry::detect(dir.path());
        assert!(result.is_empty());
    }
}
