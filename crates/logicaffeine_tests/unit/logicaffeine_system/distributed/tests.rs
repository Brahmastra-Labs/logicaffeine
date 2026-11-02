use super::*;
use logicaffeine_data::crdt::GCounter;
use tempfile::TempDir;

#[tokio::test]
async fn test_distributed_mount_empty() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", None)
        .await
        .unwrap();

    assert_eq!(counter.get().await.value(), 0);
}

#[tokio::test]
async fn test_distributed_mutate() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    let counter = Distributed::<GCounter>::mount(vfs, "counter.lsf", None)
        .await
        .unwrap();

    counter.mutate(|c| c.increment(5)).await.unwrap();
    counter.mutate(|c| c.increment(3)).await.unwrap();

    assert_eq!(counter.get().await.value(), 8);
}

#[tokio::test]
async fn test_distributed_persist_and_reload() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    // Create and mutate
    {
        let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None)
            .await
            .unwrap();
        counter.mutate(|c| c.increment(10)).await.unwrap();
        assert_eq!(counter.get().await.value(), 10);
    }

    // Reload and verify
    {
        let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None)
            .await
            .unwrap();
        assert_eq!(counter.get().await.value(), 10);
    }
}

#[tokio::test]
async fn test_distributed_compaction() {
    let temp = TempDir::new().unwrap();
    let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(crate::fs::NativeVfs::new(temp.path()));

    let counter = Distributed::<GCounter>::mount(vfs.clone(), "counter.lsf", None)
        .await
        .unwrap();

    // Mutate multiple times
    for _ in 0..5 {
        counter.mutate(|c| c.increment(1)).await.unwrap();
    }
    assert_eq!(counter.entry_count().await, 5);

    // Compact
    counter.compact().await.unwrap();
    assert_eq!(counter.entry_count().await, 1);

    // Value should be preserved
    assert_eq!(counter.get().await.value(), 5);

    // Reload should still work
    let counter2 = Distributed::<GCounter>::mount(vfs, "counter.lsf", None)
        .await
        .unwrap();
    assert_eq!(counter2.get().await.value(), 5);
}
