//! RAR archives where `archive.dll` isn't (`rar.rs` reads them through Windows' build of libarchive):
//! not read, said in words, so the library builds elsewhere (the headset's panel reads tables, never
//! archives). Ours.
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::archive::Entry;

const NOT_HERE: &str = "RAR archives are read on Windows only";

pub fn list(_parts: &[PathBuf]) -> Result<Vec<Entry>, String> {
    Err(NOT_HERE.into())
}

pub fn extract(_parts: &[PathBuf], _open: &mut crate::archive::Opener) -> Result<(), String> {
    Err(NOT_HERE.into())
}

pub fn copy_out(_parts: &[PathBuf], _entry: &str, _out: &mut dyn Write, _limit: u64) -> Result<(), String> {
    Err(NOT_HERE.into())
}

/// The archive alone: its other parts matter only to reading it.
pub fn parts_of(path: &Path) -> Vec<PathBuf> {
    vec![path.to_path_buf()]
}
