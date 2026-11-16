use flate2::{write::GzEncoder, Compression};
use std::{io, path::Path};

/// Package the completed build and its source metadata for Actions.
pub fn write_build_archive(source: &Path, directory: &Path, output: &Path) -> io::Result<()> {
    let name = directory.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Missing build directory name")
    })?;
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    let encoder = GzEncoder::new(staged.as_file_mut(), Compression::default());
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    archive.append_path_with_name(source, "source.json")?;
    archive.append_dir_all(name, directory)?;
    archive.into_inner()?.finish()?;
    staged.persist(output)?;
    Ok(())
}
