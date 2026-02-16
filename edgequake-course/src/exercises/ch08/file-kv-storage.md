# Exercise: File-Based KVStorage

::: exercise
id: ch08-file-kv-storage
difficulty: intermediate
time: 30 minutes
:::

In chapter 8 you learned that EdgeQuake uses trait-based storage abstraction to
decouple business logic from storage backends. The `KVStorage` trait defines
operations like `get_by_id`, `upsert`, and `delete` -- and any struct that
implements the trait can be plugged into the system.

In this exercise you will build a **file-system-backed KVStorage adapter** that
stores each key-value pair as a JSON file on disk. This is useful for local
development, debugging, and scenarios where you want human-readable storage
without running a database.

::: objectives
thinking:
  - Understand how the adapter pattern maps abstract trait methods to concrete I/O operations
  - Recognize the trade-offs of file-based storage vs in-memory and database backends
  - Consider concurrency and atomicity limitations of filesystem operations
doing:
  - Implement the full KVStorage trait for a file-based backend
  - Use tokio::fs for non-blocking file operations
  - Map I/O errors to StorageError variants correctly
  - Write a working transition_if_status with read-modify-write semantics
:::

::: discussion
- Why does EdgeQuake define `KVStorage` as a trait rather than using a concrete type?
- When would a file-based backend be preferable to the in-memory implementation?
- What are the atomicity limitations of filesystem writes, and how could you mitigate them?
:::

::: starter file="src/main.rs"
```rust
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

// --- Error types (simplified from edgequake-storage) ---

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Storage not initialized")]
    NotInitialized,
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        StorageError::Serialization(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

// --- KVStorage trait (simplified from edgequake-storage) ---

#[async_trait]
pub trait KVStorage: Send + Sync {
    fn namespace(&self) -> &str;
    async fn initialize(&self) -> Result<()>;
    async fn finalize(&self) -> Result<()>;
    async fn get_by_id(&self, id: &str) -> Result<Option<Value>>;
    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<Value>>;
    async fn filter_keys(&self, keys: HashSet<String>) -> Result<HashSet<String>>;
    async fn upsert(&self, data: &[(String, Value)]) -> Result<()>;
    async fn delete(&self, ids: &[String]) -> Result<()>;
    async fn is_empty(&self) -> Result<bool>;
    async fn count(&self) -> Result<usize>;
    async fn keys(&self) -> Result<Vec<String>>;
    async fn clear(&self) -> Result<()>;
    async fn transition_if_status(
        &self,
        key: &str,
        expected_status: &str,
        new_status: &str,
    ) -> Result<bool>;
}

// --- Your implementation ---

/// File-based key-value storage.
///
/// Stores each record as a JSON file at:
///   {base_path}/{namespace}/{key}.json
pub struct FileKVStorage {
    base_path: PathBuf,
    namespace: String,
}

impl FileKVStorage {
    pub fn new(base_path: impl Into<PathBuf>, namespace: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
            namespace: namespace.into(),
        }
    }

    /// Return the directory path for this namespace.
    fn namespace_dir(&self) -> PathBuf {
        self.base_path.join(&self.namespace)
    }

    /// Return the file path for a given key.
    fn key_path(&self, key: &str) -> PathBuf {
        // TODO: Build the path: namespace_dir / {key}.json
        // Be careful with keys that contain path separators -- for this
        // exercise, assume keys are simple alphanumeric strings with
        // hyphens and underscores.
        todo!("Return the full path for this key")
    }
}

#[async_trait]
impl KVStorage for FileKVStorage {
    fn namespace(&self) -> &str {
        &self.namespace
    }

    async fn initialize(&self) -> Result<()> {
        // TODO: Create the namespace directory if it does not exist.
        // Use tokio::fs::create_dir_all for async directory creation.
        todo!("Create the namespace directory")
    }

    async fn finalize(&self) -> Result<()> {
        // No-op for file storage -- writes are immediate.
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<Value>> {
        // TODO: Read the JSON file at key_path(id).
        // - If the file does not exist, return Ok(None).
        // - If the file exists, read and deserialize it.
        // - Use tokio::fs::read_to_string for async reads.
        // Hint: match on io::ErrorKind::NotFound to distinguish
        // "file missing" from real I/O errors.
        todo!("Read and deserialize the JSON file")
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<Value>> {
        // TODO: Call get_by_id for each id and collect the Some values.
        todo!("Batch get by calling get_by_id in a loop")
    }

    async fn filter_keys(&self, keys: HashSet<String>) -> Result<HashSet<String>> {
        // TODO: Return the subset of keys that do NOT have a corresponding file.
        // Hint: use tokio::fs::try_exists or key_path(k).exists() to check.
        todo!("Filter keys to find missing ones")
    }

    async fn upsert(&self, data: &[(String, Value)]) -> Result<()> {
        // TODO: For each (key, value):
        // 1. Serialize value to pretty JSON.
        // 2. Write to key_path(key) using tokio::fs::write.
        // Ensure the namespace directory exists first.
        todo!("Write each key-value pair as a JSON file")
    }

    async fn delete(&self, ids: &[String]) -> Result<()> {
        // TODO: Remove the JSON file for each id.
        // Silently ignore NotFound errors (the key was already absent).
        todo!("Delete JSON files for the given keys")
    }

    async fn is_empty(&self) -> Result<bool> {
        // TODO: Check if the namespace directory is empty or does not exist.
        todo!("Check if storage is empty")
    }

    async fn count(&self) -> Result<usize> {
        // TODO: Count .json files in the namespace directory.
        todo!("Count JSON files in the namespace directory")
    }

    async fn keys(&self) -> Result<Vec<String>> {
        // TODO: List .json files in the namespace directory and return
        // the filenames without the .json extension as keys.
        todo!("List all keys from filenames")
    }

    async fn clear(&self) -> Result<()> {
        // TODO: Remove all .json files in the namespace directory.
        // Do not remove the directory itself.
        todo!("Remove all JSON files")
    }

    async fn transition_if_status(
        &self,
        key: &str,
        expected_status: &str,
        new_status: &str,
    ) -> Result<bool> {
        // TODO: Read the value for key, check if its "status" field matches
        // expected_status, and if so, update it to new_status and write back.
        // Return Ok(true) if the transition succeeded, Ok(false) otherwise.
        //
        // Note: This is NOT truly atomic on a filesystem. In production,
        // use a database with compare-and-swap semantics.
        todo!("Read-modify-write status transition")
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let storage = FileKVStorage::new(dir.path(), "documents");
    storage.initialize().await?;

    // Quick smoke test
    storage
        .upsert(&[("doc-1".to_string(), serde_json::json!({"title": "Hello"}))])
        .await?;

    let val = storage.get_by_id("doc-1").await?;
    println!("Retrieved: {:?}", val);

    println!("All tests in main passed!");
    Ok(())
}
```
:::

::: hint level=1 title="Structuring key_path"
The file path should be `self.namespace_dir().join(format!("{}.json", key))`.
Keep it simple -- for this exercise, keys are safe filename strings.
:::

::: hint level=2 title="Handling missing files in get_by_id"
Use a `match` on the error kind:
```rust
match tokio::fs::read_to_string(path).await {
    Ok(contents) => {
        let value: Value = serde_json::from_str(&contents)?;
        Ok(Some(value))
    }
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(e) => Err(StorageError::Io(e)),
}
```
:::

::: hint level=3 title="Listing keys from the directory"
```rust
let mut keys = Vec::new();
let mut entries = tokio::fs::read_dir(self.namespace_dir()).await?;
while let Some(entry) = entries.next_entry().await? {
    if let Some(name) = entry.file_name().to_str() {
        if let Some(key) = name.strip_suffix(".json") {
            keys.push(key.to_string());
        }
    }
}
Ok(keys)
```
:::

::: solution
```rust
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Storage not initialized")]
    NotInitialized,
}

impl From<serde_json::Error> for StorageError {
    fn from(err: serde_json::Error) -> Self {
        StorageError::Serialization(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[async_trait]
pub trait KVStorage: Send + Sync {
    fn namespace(&self) -> &str;
    async fn initialize(&self) -> Result<()>;
    async fn finalize(&self) -> Result<()>;
    async fn get_by_id(&self, id: &str) -> Result<Option<Value>>;
    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<Value>>;
    async fn filter_keys(&self, keys: HashSet<String>) -> Result<HashSet<String>>;
    async fn upsert(&self, data: &[(String, Value)]) -> Result<()>;
    async fn delete(&self, ids: &[String]) -> Result<()>;
    async fn is_empty(&self) -> Result<bool>;
    async fn count(&self) -> Result<usize>;
    async fn keys(&self) -> Result<Vec<String>>;
    async fn clear(&self) -> Result<()>;
    async fn transition_if_status(
        &self,
        key: &str,
        expected_status: &str,
        new_status: &str,
    ) -> Result<bool>;
}

pub struct FileKVStorage {
    base_path: PathBuf,
    namespace: String,
}

impl FileKVStorage {
    pub fn new(base_path: impl Into<PathBuf>, namespace: impl Into<String>) -> Self {
        Self {
            base_path: base_path.into(),
            namespace: namespace.into(),
        }
    }

    fn namespace_dir(&self) -> PathBuf {
        self.base_path.join(&self.namespace)
    }

    fn key_path(&self, key: &str) -> PathBuf {
        self.namespace_dir().join(format!("{}.json", key))
    }
}

#[async_trait]
impl KVStorage for FileKVStorage {
    fn namespace(&self) -> &str {
        &self.namespace
    }

    async fn initialize(&self) -> Result<()> {
        tokio::fs::create_dir_all(self.namespace_dir()).await?;
        Ok(())
    }

    async fn finalize(&self) -> Result<()> {
        Ok(())
    }

    async fn get_by_id(&self, id: &str) -> Result<Option<Value>> {
        let path = self.key_path(id);
        match tokio::fs::read_to_string(&path).await {
            Ok(contents) => {
                let value: Value = serde_json::from_str(&contents)?;
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(e)),
        }
    }

    async fn get_by_ids(&self, ids: &[String]) -> Result<Vec<Value>> {
        let mut results = Vec::new();
        for id in ids {
            if let Some(value) = self.get_by_id(id).await? {
                results.push(value);
            }
        }
        Ok(results)
    }

    async fn filter_keys(&self, keys: HashSet<String>) -> Result<HashSet<String>> {
        let mut missing = HashSet::new();
        for key in keys {
            let path = self.key_path(&key);
            if !path.exists() {
                missing.insert(key);
            }
        }
        Ok(missing)
    }

    async fn upsert(&self, data: &[(String, Value)]) -> Result<()> {
        // Ensure directory exists
        tokio::fs::create_dir_all(self.namespace_dir()).await?;

        for (key, value) in data {
            let path = self.key_path(key);
            let json = serde_json::to_string_pretty(value)?;
            tokio::fs::write(&path, json).await?;
        }
        Ok(())
    }

    async fn delete(&self, ids: &[String]) -> Result<()> {
        for id in ids {
            let path = self.key_path(id);
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(StorageError::Io(e)),
            }
        }
        Ok(())
    }

    async fn is_empty(&self) -> Result<bool> {
        let dir = self.namespace_dir();
        if !dir.exists() {
            return Ok(true);
        }
        let mut entries = tokio::fs::read_dir(&dir).await?;
        Ok(entries.next_entry().await?.is_none())
    }

    async fn count(&self) -> Result<usize> {
        let dir = self.namespace_dir();
        if !dir.exists() {
            return Ok(0);
        }
        let mut count = 0;
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".json") {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    async fn keys(&self) -> Result<Vec<String>> {
        let dir = self.namespace_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if let Some(key) = name.strip_suffix(".json") {
                    keys.push(key.to_string());
                }
            }
        }
        Ok(keys)
    }

    async fn clear(&self) -> Result<()> {
        let dir = self.namespace_dir();
        if !dir.exists() {
            return Ok(());
        }
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if let Some(name) = entry.file_name().to_str() {
                if name.ends_with(".json") {
                    tokio::fs::remove_file(entry.path()).await?;
                }
            }
        }
        Ok(())
    }

    async fn transition_if_status(
        &self,
        key: &str,
        expected_status: &str,
        new_status: &str,
    ) -> Result<bool> {
        // Read current value
        let value = match self.get_by_id(key).await? {
            Some(v) => v,
            None => return Ok(false),
        };

        // Check current status
        let current_status = value.get("status").and_then(|v| v.as_str());
        if current_status != Some(expected_status) {
            return Ok(false);
        }

        // Update status and write back
        let mut obj = value;
        if let Some(map) = obj.as_object_mut() {
            map.insert("status".to_string(), serde_json::json!(new_status));
        }

        let path = self.key_path(key);
        let json = serde_json::to_string_pretty(&obj)?;
        tokio::fs::write(&path, json).await?;

        Ok(true)
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let storage = FileKVStorage::new(dir.path(), "documents");
    storage.initialize().await?;

    storage
        .upsert(&[("doc-1".to_string(), serde_json::json!({"title": "Hello"}))])
        .await?;

    let val = storage.get_by_id("doc-1").await?;
    println!("Retrieved: {:?}", val);

    println!("All tests in main passed!");
    Ok(())
}
```

### Explanation

The `FileKVStorage` maps each KVStorage operation to filesystem calls:

- **key_path**: Each key becomes `{base_path}/{namespace}/{key}.json`.
- **get_by_id**: Reads the file and deserializes JSON. Returns `Ok(None)` for missing
  files instead of propagating a `NotFound` error.
- **upsert**: Serializes to pretty JSON and writes the file. Uses `create_dir_all`
  defensively.
- **delete**: Removes the file, ignoring `NotFound` errors for idempotency.
- **count/keys/clear**: Use `read_dir` to iterate `.json` files in the namespace directory.
- **transition_if_status**: Performs a read-modify-write cycle. This is NOT atomic --
  in production, use a database with compare-and-swap semantics (like DynamoDB
  conditional writes or PostgreSQL's `UPDATE ... WHERE status = $expected`).
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn setup() -> (tempfile::TempDir, FileKVStorage) {
        let dir = tempfile::tempdir().unwrap();
        let storage = FileKVStorage::new(dir.path(), "test");
        storage.initialize().await.unwrap();
        (dir, storage)
    }

    #[tokio::test]
    async fn test_upsert_and_get() {
        let (_dir, storage) = setup().await;

        let value = json!({"title": "Test Document", "version": 1});
        storage
            .upsert(&[("doc-1".to_string(), value.clone())])
            .await
            .unwrap();

        let retrieved = storage.get_by_id("doc-1").await.unwrap();
        assert_eq!(retrieved, Some(value));
    }

    #[tokio::test]
    async fn test_get_missing_returns_none() {
        let (_dir, storage) = setup().await;

        let result = storage.get_by_id("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_get_by_ids() {
        let (_dir, storage) = setup().await;

        storage
            .upsert(&[
                ("a".to_string(), json!(1)),
                ("b".to_string(), json!(2)),
                ("c".to_string(), json!(3)),
            ])
            .await
            .unwrap();

        let ids = vec!["a".to_string(), "c".to_string(), "missing".to_string()];
        let results = storage.get_by_ids(&ids).await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_delete() {
        let (_dir, storage) = setup().await;

        storage
            .upsert(&[("doc-1".to_string(), json!({"x": 1}))])
            .await
            .unwrap();
        assert_eq!(storage.count().await.unwrap(), 1);

        storage.delete(&["doc-1".to_string()]).await.unwrap();
        assert_eq!(storage.count().await.unwrap(), 0);
        assert!(storage.get_by_id("doc-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_is_ok() {
        let (_dir, storage) = setup().await;
        // Should not error
        storage.delete(&["ghost".to_string()]).await.unwrap();
    }

    #[tokio::test]
    async fn test_count_and_keys() {
        let (_dir, storage) = setup().await;

        assert_eq!(storage.count().await.unwrap(), 0);
        assert!(storage.is_empty().await.unwrap());

        storage
            .upsert(&[
                ("alpha".to_string(), json!(1)),
                ("beta".to_string(), json!(2)),
            ])
            .await
            .unwrap();

        assert_eq!(storage.count().await.unwrap(), 2);
        assert!(!storage.is_empty().await.unwrap());

        let mut keys = storage.keys().await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn test_filter_keys() {
        let (_dir, storage) = setup().await;

        storage
            .upsert(&[("a".to_string(), json!(1)), ("b".to_string(), json!(2))])
            .await
            .unwrap();

        let to_check: HashSet<String> =
            ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let missing = storage.filter_keys(to_check).await.unwrap();

        assert_eq!(missing.len(), 2);
        assert!(missing.contains("c"));
        assert!(missing.contains("d"));
    }

    #[tokio::test]
    async fn test_clear() {
        let (_dir, storage) = setup().await;

        storage
            .upsert(&[
                ("x".to_string(), json!(1)),
                ("y".to_string(), json!(2)),
            ])
            .await
            .unwrap();
        assert_eq!(storage.count().await.unwrap(), 2);

        storage.clear().await.unwrap();
        assert_eq!(storage.count().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_upsert_overwrites() {
        let (_dir, storage) = setup().await;

        storage
            .upsert(&[("doc-1".to_string(), json!({"version": 1}))])
            .await
            .unwrap();
        storage
            .upsert(&[("doc-1".to_string(), json!({"version": 2}))])
            .await
            .unwrap();

        let val = storage.get_by_id("doc-1").await.unwrap().unwrap();
        assert_eq!(val["version"], 2);
    }

    #[tokio::test]
    async fn test_transition_if_status_success() {
        let (_dir, storage) = setup().await;

        let doc = json!({"id": "doc-1", "status": "failed", "title": "Test"});
        storage
            .upsert(&[("doc-1".to_string(), doc)])
            .await
            .unwrap();

        let result = storage
            .transition_if_status("doc-1", "failed", "deleting")
            .await
            .unwrap();
        assert!(result);

        let updated = storage.get_by_id("doc-1").await.unwrap().unwrap();
        assert_eq!(updated["status"], "deleting");
    }

    #[tokio::test]
    async fn test_transition_if_status_wrong_status() {
        let (_dir, storage) = setup().await;

        let doc = json!({"id": "doc-1", "status": "processing"});
        storage
            .upsert(&[("doc-1".to_string(), doc)])
            .await
            .unwrap();

        let result = storage
            .transition_if_status("doc-1", "failed", "deleting")
            .await
            .unwrap();
        assert!(!result);

        let unchanged = storage.get_by_id("doc-1").await.unwrap().unwrap();
        assert_eq!(unchanged["status"], "processing");
    }

    #[tokio::test]
    async fn test_transition_if_status_missing_key() {
        let (_dir, storage) = setup().await;

        let result = storage
            .transition_if_status("nonexistent", "failed", "deleting")
            .await
            .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_namespace() {
        let (_dir, storage) = setup().await;
        assert_eq!(storage.namespace(), "test");
    }
}
```
:::

::: reflection
- How does the file-per-key approach compare to storing all keys in a single file?
  What are the trade-offs for concurrent access?
- The `transition_if_status` method is not truly atomic on a filesystem. What could
  go wrong in a multi-process scenario? How would you fix it?
- If you needed to support keys containing `/` or other path-unsafe characters,
  what encoding scheme would you use?
:::
