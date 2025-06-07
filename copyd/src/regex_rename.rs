use anyhow::{Result, Context};
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::{info, debug, warn};

/// Handles regex-based file renaming during copy operations
pub struct RegexRenamer {
    pattern: Option<Regex>,
    replacement: String,
}

impl RegexRenamer {
    /// Create a new regex renamer with pattern and replacement
    pub fn new(pattern: &str, replacement: &str) -> Result<Self> {
        let regex = if pattern.is_empty() {
            None
        } else {
            Some(Regex::new(pattern)
                .with_context(|| format!("Invalid regex pattern: {}", pattern))?)
        };

        Ok(RegexRenamer {
            pattern: regex,
            replacement: replacement.to_string(),
        })
    }

    /// Create a disabled regex renamer (no transformations)
    pub fn disabled() -> Self {
        RegexRenamer {
            pattern: None,
            replacement: String::new(),
        }
    }

    /// Check if regex renaming is enabled
    pub fn is_enabled(&self) -> bool {
        self.pattern.is_some() && !self.replacement.is_empty()
    }

    /// Transform a file path according to the regex pattern
    pub fn transform_path(&self, original_path: &Path, base_destination: &Path) -> Result<PathBuf> {
        if !self.is_enabled() {
            // No transformation, use original logic
            return Ok(base_destination.to_path_buf());
        }

        let pattern = self.pattern.as_ref().expect("RegexRenamer::transform_path called without pattern");
        
        // Extract the filename for transformation
        let original_name = original_path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid filename in path: {:?}", original_path))?;

        // Apply regex transformation
        let new_name = pattern.replace_all(original_name, &self.replacement);
        
        if new_name == original_name {
            debug!("No regex match for filename: {}", original_name);
            return Ok(base_destination.to_path_buf());
        }

        info!("Regex rename: '{}' -> '{}'", original_name, new_name);

        // Construct new destination path
        if base_destination.is_dir() {
            // If destination is a directory, place renamed file inside it
            Ok(base_destination.join(new_name.as_ref()))
        } else {
            // If destination is a file path, replace the filename part
            let parent = base_destination.parent()
                .unwrap_or_else(|| Path::new(""));
            Ok(parent.join(new_name.as_ref()))
        }
    }

    /// Transform multiple paths in a batch operation
    pub fn transform_paths(&self, source_paths: &[PathBuf], base_destination: &Path) -> Result<Vec<(PathBuf, PathBuf)>> {
        let mut transformed_paths = Vec::new();

        for source_path in source_paths {
            let dest_path = self.transform_path(source_path, base_destination)?;
            transformed_paths.push((source_path.clone(), dest_path));
        }

        Ok(transformed_paths)
    }

    /// Preview regex transformations without performing them
    pub fn preview_transformations(&self, source_paths: &[PathBuf], base_destination: &Path) -> Result<Vec<RegexTransformPreview>> {
        let mut previews = Vec::new();

        for source_path in source_paths {
            let original_name = match source_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name,
                None => {
                    previews.push(RegexTransformPreview {
                        source_path: source_path.clone(),
                        original_name: "<invalid>".into(),
                        new_name: "<invalid>".into(),
                        new_path: base_destination.to_path_buf(),
                        would_change: false,
                    });
                    continue;
                }
            };

            let (new_name, would_change) = if let Some(pattern) = &self.pattern {
                let transformed = pattern.replace_all(original_name, &self.replacement);
                let changed = transformed != original_name;
                (transformed.to_string(), changed)
            } else {
                (original_name.to_string(), false)
            };

            let new_path = if would_change {
                if base_destination.is_dir() {
                    base_destination.join(&new_name)
                } else {
                    let parent = base_destination.parent().unwrap_or_else(|| Path::new(""));
                    parent.join(&new_name)
                }
            } else {
                base_destination.to_path_buf()
            };

            previews.push(RegexTransformPreview {
                source_path: source_path.clone(),
                original_name: original_name.to_string(),
                new_name,
                new_path,
                would_change,
            });
        }

        Ok(previews)
    }

    /// Validate that the regex pattern and replacement are safe
    pub fn validate(&self) -> Result<()> {
        if let Some(pattern) = &self.pattern {
            // Test the pattern with a sample filename
            let test_cases = ["file.txt", "document.pdf", "image.jpg", "archive.tar.gz"];
            
            for test_case in &test_cases {
                match pattern.replace_all(test_case, &self.replacement) {
                    result if result.is_empty() => {
                        return Err(anyhow::anyhow!(
                            "Regex replacement results in empty filename for '{}'. This could cause data loss.",
                            test_case
                        ));
                    }
                    result if result.contains('/') || result.contains('\\') => {
                        return Err(anyhow::anyhow!(
                            "Regex replacement contains path separators for '{}': '{}'. This is not allowed.",
                            test_case, result
                        ));
                    }
                    result if result.starts_with('.') && result.len() <= 2 => {
                        warn!("Regex replacement creates hidden file for '{}': '{}'", test_case, result);
                    }
                    _ => {} // Valid result
                }
            }
        }

        Ok(())
    }

    /// Get the regex pattern as a string (for display/logging)
    pub fn pattern_str(&self) -> &str {
        if let Some(pattern) = &self.pattern {
            pattern.as_str()
        } else {
            ""
        }
    }

    /// Get the replacement string (for display/logging)
    pub fn replacement_str(&self) -> &str {
        &self.replacement
    }
}

/// Preview of a regex transformation
#[derive(Debug, Clone)]
pub struct RegexTransformPreview {
    pub source_path: PathBuf,
    pub original_name: String,
    pub new_name: String,
    pub new_path: PathBuf,
    pub would_change: bool,
}

impl RegexTransformPreview {
    pub fn display(&self) -> String {
        if self.would_change {
            format!("{:?} -> {:?}", self.source_path, self.new_path)
        } else {
            format!("{:?} (unchanged)", self.source_path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_regex_renamer_basic() -> Result<()> {
        let renamer = RegexRenamer::new(r"\.txt$", ".bak")?;
        assert!(renamer.is_enabled());
        
        let source = Path::new("file.txt");
        let dest_dir = Path::new("/tmp/dest");
        let result = renamer.transform_path(source, dest_dir)?;
        
        assert_eq!(result, Path::new("/tmp/dest/file.bak"));
        Ok(())
    }

    #[test]
    fn test_regex_renamer_no_match() -> Result<()> {
        let renamer = RegexRenamer::new(r"\.txt$", ".bak")?;
        
        let source = Path::new("file.pdf");
        let dest_dir = Path::new("/tmp/dest");
        let result = renamer.transform_path(source, dest_dir)?;
        
        assert_eq!(result, dest_dir);
        Ok(())
    }

    #[test]
    fn test_regex_renamer_disabled() -> Result<()> {
        let renamer = RegexRenamer::disabled();
        assert!(!renamer.is_enabled());
        
        let source = Path::new("file.txt");
        let dest_dir = Path::new("/tmp/dest");
        let result = renamer.transform_path(source, dest_dir)?;
        
        assert_eq!(result, dest_dir);
        Ok(())
    }

    #[test]
    fn test_regex_renamer_complex_pattern() -> Result<()> {
        // Pattern to add timestamp prefix
        let renamer = RegexRenamer::new(r"^(.+)\.(.+)$", "2024_$1.$2")?;
        
        let source = Path::new("document.pdf");
        let dest_dir = Path::new("/tmp/dest");
        let result = renamer.transform_path(source, dest_dir)?;
        
        assert_eq!(result, Path::new("/tmp/dest/2024_document.pdf"));
        Ok(())
    }

    #[test]
    fn test_regex_renamer_preview() -> Result<()> {
        let renamer = RegexRenamer::new(r"(\d+)", "num_$1")?;
        
        let sources = vec![
            PathBuf::from("file1.txt"),
            PathBuf::from("document.pdf"),
            PathBuf::from("image42.jpg"),
        ];
        
        let dest_dir = Path::new("/tmp/dest");
        let previews = renamer.preview_transformations(&sources, dest_dir)?;
        
        assert_eq!(previews.len(), 3);
        assert_eq!(previews[0].new_name, "filenum_1.txt");
        assert_eq!(previews[1].new_name, "document.pdf");
        assert_eq!(previews[2].new_name, "imagenum_42.jpg");
        
        assert!(previews[0].would_change);
        assert!(!previews[1].would_change);
        assert!(previews[2].would_change);
        
        Ok(())
    }

    #[test]
    fn test_regex_renamer_validation() -> Result<()> {
        // Valid pattern should pass
        let valid_renamer = RegexRenamer::new(r"\.txt$", ".bak")?;
        assert!(valid_renamer.validate().is_ok());
        
        // Pattern that creates empty names should fail
        let empty_renamer = RegexRenamer::new(r".*", "")?;
        assert!(empty_renamer.validate().is_err());
        
        // Pattern that creates path separators should fail
        let path_renamer = RegexRenamer::new(r"(.*)", "subdir/$1")?;
        assert!(path_renamer.validate().is_err());
        
        Ok(())
    }

    #[test]
    fn test_invalid_regex_pattern() {
        let result = RegexRenamer::new(r"[invalid", "replacement");
        assert!(result.is_err());
    }
} 