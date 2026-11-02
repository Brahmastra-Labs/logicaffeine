use super::*;
use logicaffeine_data::crdt::GCounter;
use tempfile::TempDir;

#[tokio::test]
async fn test_persistent_mount_empty() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    let counter = Persistent::<GCounter>::mount(vfs, "counter.journal")
        .await
        .unwrap();

    assert_eq!(counter.get().await.value(), 0);
}

#[tokio::test]
async fn test_persistent_mutate() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    let counter = Persistent::<GCounter>::mount(vfs, "counter.journal")
        .await
        .unwrap();

    counter.mutate(|c| c.increment(5)).await.unwrap();
    counter.mutate(|c| c.increment(3)).await.unwrap();

    assert_eq!(counter.get().await.value(), 8);
}
