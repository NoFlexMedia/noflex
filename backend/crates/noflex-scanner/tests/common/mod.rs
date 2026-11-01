// SPDX-License-Identifier: AGPL-3.0-only
//
// Fixture helpers shared by scan_movies and scan_shows integration tests.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

use noflex_core::LibraryId;
use noflex_storage::{
    Storage,
    domain::{Library, LibraryKind},
};
use noflex_storage_turso::TursoStore;
use tempfile::TempDir;

/// Owned bundle of test scaffolding. Drop the `_db_dir` and `_lib_dir`
/// keeps tempdirs alive for the test's duration.
pub struct Harness {
    pub lib_root: PathBuf,
    pub library_id: LibraryId,
    pub storage: Arc<dyn Storage>,
    _db_dir: TempDir,
    _lib_dir: TempDir,
}

pub async fn build(kind: LibraryKind) -> Harness {
    let db_dir = TempDir::new().expect("tempdir db");
    let lib_dir = TempDir::new().expect("tempdir lib");
    let store = TursoStore::open(db_dir.path().join("scanner.db"))
        .await
        .expect("open turso");
    store.migrate().await.expect("migrate");

    let library = Library {
        id: LibraryId::new(),
        name: format!("Test {kind:?}"),
        kind,
        paths: vec![lib_dir.path().to_string_lossy().into_owned()],
        ..Default::default()
    };
    store.upsert_library(&library).await.expect("upsert lib");

    Harness {
        lib_root: lib_dir.path().to_path_buf(),
        library_id: library.id,
        storage: Arc::new(store),
        _db_dir: db_dir,
        _lib_dir: lib_dir,
    }
}

/// Create a file with deterministic, non-empty content so size_bytes differs
/// across rewrites. Creates parent directories as needed.
pub fn touch(root: &Path, rel: &str, content: &[u8]) -> PathBuf {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir -p");
    }
    let mut f = fs::File::create(&path).expect("create file");
    f.write_all(content).expect("write file");
    f.sync_all().expect("sync");
    path
}

/// Delete a file. Panics if it doesn't exist (we only call this when we know
/// the fixture put it there).
pub fn unlink(root: &Path, rel: &str) {
    let path = root.join(rel);
    fs::remove_file(&path).expect("remove file");
}
