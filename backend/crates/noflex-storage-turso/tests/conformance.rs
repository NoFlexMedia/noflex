// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage_turso::TursoStore;
use tempfile::TempDir;

#[tokio::test]
async fn conformance() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("conformance.db");
    let store = TursoStore::open(&db_path).await.expect("open");
    noflex_storage::test_suite::run_all(&store).await;
}
