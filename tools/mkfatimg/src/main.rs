//! mkfatimg — Create a FAT32 disk image populated with host files.
//!
//! Usage: mkfatimg <output.img> <size_mb> [options] [host_path:target_path ...]
//!
//! Options:
//!   --volume-id <hex8>   Set the FAT32 Volume Serial Number (8 hex digits,
//!                        e.g. BAZ70001).  Default: BAZZULTO label, random ID.
//!   --label <label>      Set the 11-byte FAT32 volume label (default: BAZZULTO).
//!                        Padded with spaces if shorter than 11 characters.
//!
//! Creates a FAT32-formatted raw disk image, then copies each host file into
//! the image at its target path, creating parent directories as needed.
//! Pure Rust — no external tools required.

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use std::fs::OpenOptions;
use std::io::{self, Seek, SeekFrom, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let output = args.get(1).map(String::as_str).unwrap_or("disk.img");
    let size_mb: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(256);

    // Parse optional flags and remaining args.
    let mut volume_id: Option<u32> = None;
    let mut volume_label: [u8; 11] = *b"BAZZULTO   ";
    let mut mappings: Vec<(String, String)> = Vec::new();
    let mut dir_only: Vec<String> = Vec::new();
    let mut tree_mappings: Vec<(String, String)> = Vec::new();

    let mut iter = args[3..].iter();
    while let Some(arg) = iter.next() {
        if arg == "--volume-id" {
            if let Some(hex) = iter.next() {
                volume_id = u32::from_str_radix(hex, 16).ok();
                if volume_id.is_none() {
                    eprintln!("mkfatimg: invalid --volume-id value '{}'", hex);
                    std::process::exit(1);
                }
            }
        } else if arg == "--label" {
            if let Some(label) = iter.next() {
                let bytes = label.as_bytes();
                volume_label = [b' '; 11];
                let copy_len = bytes.len().min(11);
                volume_label[..copy_len].copy_from_slice(&bytes[..copy_len]);
            }
        } else if let Some(target) = arg.strip_prefix("DIR:") {
            dir_only.push(target.to_string());
        } else if let Some(rest) = arg.strip_prefix("TREE:") {
            // TREE:host_dir:target_dir — copy entire host directory tree recursively.
            if let Some(colon) = rest.find(':') {
                tree_mappings.push((rest[..colon].to_string(), rest[colon + 1..].to_string()));
            } else {
                eprintln!("mkfatimg: invalid TREE mapping '{}' (expected TREE:host:target)", arg);
                std::process::exit(1);
            }
        } else if let Some(colon) = arg.find(':') {
            mappings.push((arg[..colon].to_string(), arg[colon + 1..].to_string()));
        }
    }

    if let Err(e) = create_image(output, size_mb, volume_id, volume_label, &mappings, &dir_only, &tree_mappings) {
        eprintln!("mkfatimg: {}", e);
        std::process::exit(1);
    }
}

fn create_image(
    output: &str,
    size_mb: u64,
    volume_id: Option<u32>,
    volume_label: [u8; 11],
    mappings: &[(String, String)],
    dir_only: &[String],
    tree_mappings: &[(String, String)],
) -> io::Result<()> {
    println!("mkfatimg: creating {}MB FAT32 image at {}", size_mb, output);

    // Create a sparse file of the target size.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)?;
    file.set_len(size_mb * 1024 * 1024)?;

    // Format as FAT32.
    // With a large enough image (>= 64 MiB at default cluster size), fatfs
    // automatically selects FAT32 based on cluster count.
    let mut format_options = FormatVolumeOptions::new()
        .fat_type(FatType::Fat32)
        .volume_label(volume_label);
    if let Some(id) = volume_id {
        format_options = format_options.volume_id(id);
    }
    fatfs::format_volume(&mut file, format_options)?;

    // Reopen the filesystem.
    file.seek(SeekFrom::Start(0))?;
    let fs = FileSystem::new(file, FsOptions::new())?;
    let root = fs.root_dir();

    // Copy each file into the image.
    let mut written = 0usize;
    for (host_path, target_path) in mappings {
        match install_file(&root, host_path, target_path) {
            Ok(()) => written += 1,
            Err(e) => eprintln!("mkfatimg: warning: {} -> {}: {}", host_path, target_path, e),
        }
    }

    // Create any DIR-only entries (empty directories).
    for dir_path in dir_only {
        let target = dir_path.trim_start_matches('/');
        let components: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
        for depth in 0..components.len() {
            ensure_dir(&root, &components[..=depth])?;
        }
        println!("  /{} (directory)", target);
    }

    // Copy entire host directory trees (TREE:host_dir:target_dir).
    for (host_dir, target_dir) in tree_mappings {
        match install_tree(&root, host_dir, target_dir) {
            Ok(count) => written += count,
            Err(e) => eprintln!("mkfatimg: warning: TREE {} -> {}: {}", host_dir, target_dir, e),
        }
    }

    println!("mkfatimg: done ({}/{} files written)", written, mappings.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// install_file — copy one host file into the FAT32 image at target_path
// ---------------------------------------------------------------------------

fn install_file<'a, IO: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'a, IO>,
    host_path: &str,
    target_path: &str,
) -> io::Result<()> {
    let target = target_path.trim_start_matches('/');

    // Split into directory components and the final filename.
    let (dir_path, filename) = match target.rfind('/') {
        Some(i) => (&target[..i], &target[i + 1..]),
        None    => ("", target),
    };
    let dir_components: Vec<&str> =
        dir_path.split('/').filter(|s| !s.is_empty()).collect();

    // Read the host file.
    let data = std::fs::read(host_path).map_err(|e| {
        io::Error::new(e.kind(), format!("cannot read '{}': {}", host_path, e))
    })?;

    // Ensure each directory level exists, from shallowest to deepest.
    for depth in 0..dir_components.len() {
        ensure_dir(root, &dir_components[..=depth])?;
    }

    // Open the target directory (or use root if no subdirectory) and write.
    if dir_components.is_empty() {
        let _ = root.remove(filename); // overwrite if exists
        let mut f = root.create_file(filename)?;
        f.write_all(&data)?;
    } else {
        let dir = open_dir_from_root(root, &dir_components)?;
        let _ = dir.remove(filename); // overwrite if exists
        let mut f = dir.create_file(filename)?;
        f.write_all(&data)?;
    }

    println!("  /{} ({} bytes)", target, data.len());
    Ok(())
}

// ---------------------------------------------------------------------------
// ensure_dir — create the directory at components[last] if it does not exist.
//
// All parent components (components[..last]) must already exist.
// ---------------------------------------------------------------------------

fn ensure_dir<'a, IO: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'a, IO>,
    components: &[&str],
) -> io::Result<()> {
    let (&leaf, parents) = match components.split_last() {
        Some(r) => r,
        None    => return Ok(()),
    };

    let result = if parents.is_empty() {
        root.create_dir(leaf)
    } else {
        let parent = open_dir_from_root(root, parents)?;
        parent.create_dir(leaf)
    };

    match result {
        Ok(_)                                               => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e)                                              => Err(e),
    }
}

// ---------------------------------------------------------------------------
// open_dir_from_root — navigate from root down the given path components.
//
// All components must already exist. `components` must be non-empty.
//
// Each Dir<'a, IO> returned by open_dir borrows only from the FileSystem
// (lifetime 'a), not from the parent Dir, so rebinding `current` in the
// loop is safe: the temporary borrow of the old `current` ends before the
// new value is assigned.
// ---------------------------------------------------------------------------

fn open_dir_from_root<'a, IO: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'a, IO>,
    components: &[&str],
) -> io::Result<fatfs::Dir<'a, IO>> {
    let mut current = root.open_dir(components[0])?;
    for &component in &components[1..] {
        current = current.open_dir(component)?;
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// install_tree — recursively copy a host directory tree into the image.
//
// Returns the number of files written.
// ---------------------------------------------------------------------------

fn install_tree<'a, IO: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'a, IO>,
    host_dir: &str,
    target_dir: &str,
) -> io::Result<usize> {
    let mut count = 0usize;
    install_tree_recursive(root, std::path::Path::new(host_dir), target_dir, &mut count)?;
    Ok(count)
}

fn install_tree_recursive<'a, IO: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'a, IO>,
    host_path: &std::path::Path,
    target_prefix: &str,
    count: &mut usize,
) -> io::Result<()> {
    let entries = std::fs::read_dir(host_path)?;
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let target_path = format!("{}/{}", target_prefix.trim_end_matches('/'), name_str);

        if file_type.is_dir() {
            install_tree_recursive(root, &entry.path(), &target_path, count)?;
        } else if file_type.is_file() {
            match install_file(root, entry.path().to_str().unwrap_or(""), &target_path) {
                Ok(()) => *count += 1,
                Err(e) => eprintln!("mkfatimg: warning: {}: {}", target_path, e),
            }
        }
        // Symlinks are skipped.
    }
    Ok(())
}
