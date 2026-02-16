# Exercise: Batch State Machine

::: exercise
id: ch12-batch-state-machine
difficulty: advanced
time: 30 minutes
:::

In chapter 12 you learned that EdgeQuake's batch ingestion pipeline has four
phases: Prepare, Extract, Embed, and Store. Each phase can fail independently,
and the system must be able to resume from the last successful checkpoint after
a crash. This requires a well-defined state machine with validated transitions.

In this exercise you will implement the batch processing state machine: a
`Phase` enum with validated transitions, a `BatchProcessor` that advances
through phases with checkpoint persistence, and resume logic that skips
already-completed phases.

::: objectives
thinking:
  - Understand finite state machines and why they make distributed pipelines predictable
  - Recognize the importance of checkpointing for crash recovery
  - Distinguish between active phases (processing) and completed phases (checkpoints)
doing:
  - Implement valid phase transition rules
  - Build a BatchProcessor that advances through phases and saves checkpoints
  - Add resume logic that loads the last checkpoint and continues
  - Handle failure states correctly
:::

::: discussion
- Why does EdgeQuake use explicit phase states (Preparing, Prepared) instead of just four states (Prepare, Extract, Embed, Store)?
- What would happen if the system crashed between a phase completing and the checkpoint being saved?
- How does idempotency in each phase protect against data corruption during resume?
:::

::: starter file="src/main.rs"
```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

// --- Phase enum (modeled after edgequake-batch) ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Pending,
    Preparing,
    Prepared,
    Extracting,
    Extracted,
    Embedding,
    Embedded,
    Storing,
    Completed,
    Failed,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Phase::Pending => write!(f, "pending"),
            Phase::Preparing => write!(f, "preparing"),
            Phase::Prepared => write!(f, "prepared"),
            Phase::Extracting => write!(f, "extracting"),
            Phase::Extracted => write!(f, "extracted"),
            Phase::Embedding => write!(f, "embedding"),
            Phase::Embedded => write!(f, "embedded"),
            Phase::Storing => write!(f, "storing"),
            Phase::Completed => write!(f, "completed"),
            Phase::Failed => write!(f, "failed"),
        }
    }
}

// --- Job State ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: String,
    pub phase: Phase,
    pub total_documents: usize,
    pub processed_documents: usize,
    pub error: Option<String>,
    pub updated_at: String,
}

// --- Simple KV store for checkpoints (simulates DynamoDB) ---

pub struct MemoryStore {
    data: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    pub fn save(&self, key: &str, value: &str) {
        self.data.lock().unwrap().insert(key.to_string(), value.to_string());
    }

    pub fn load(&self, key: &str) -> Option<String> {
        self.data.lock().unwrap().get(key).cloned()
    }
}

// --- Errors ---

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("Invalid phase transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Job is in a terminal state: {0}")]
    TerminalState(String),

    #[error("Checkpoint error: {0}")]
    CheckpointError(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),
}

pub type Result<T> = std::result::Result<T, BatchError>;

// --- Phase transition logic ---

/// Check whether a transition from `from` to `to` is valid.
///
/// Valid transitions follow the pipeline order:
///   Pending -> Preparing -> Prepared -> Extracting -> Extracted ->
///   Embedding -> Embedded -> Storing -> Completed
///
/// Additionally, any active phase (Preparing, Extracting, Embedding,
/// Storing) can transition to Failed.
pub fn is_valid_transition(from: &Phase, to: &Phase) -> bool {
    // TODO: Implement the transition rules.
    //
    // The valid transitions are:
    //   Pending     -> Preparing
    //   Preparing   -> Prepared | Failed
    //   Prepared    -> Extracting
    //   Extracting  -> Extracted | Failed
    //   Extracted   -> Embedding
    //   Embedding   -> Embedded | Failed
    //   Embedded    -> Storing
    //   Storing     -> Completed | Failed
    //
    // All other transitions are invalid.
    // Failed and Completed are terminal -- no transitions out.
    todo!("Implement phase transition validation")
}

// --- Batch Processor ---

pub struct BatchProcessor {
    store: MemoryStore,
    state: JobState,
}

impl BatchProcessor {
    /// Create a new batch processor for a fresh job.
    pub fn new(job_id: &str, store: MemoryStore) -> Self {
        let state = JobState {
            job_id: job_id.to_string(),
            phase: Phase::Pending,
            total_documents: 0,
            processed_documents: 0,
            error: None,
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        Self { store, state }
    }

    /// Get the current job state.
    pub fn state(&self) -> &JobState {
        &self.state
    }

    /// Advance to the next phase.
    ///
    /// Validates the transition, updates the state, and saves a checkpoint.
    pub fn advance_to(&mut self, next_phase: Phase) -> Result<()> {
        // TODO:
        // 1. Check if the transition is valid using is_valid_transition().
        // 2. If invalid, return BatchError::InvalidTransition.
        // 3. Update self.state.phase to next_phase.
        // 4. Update self.state.updated_at to the current time.
        // 5. Save a checkpoint using self.save_checkpoint().
        todo!("Validate transition, update state, save checkpoint")
    }

    /// Mark the job as failed with an error message.
    pub fn fail(&mut self, error: &str) -> Result<()> {
        // TODO:
        // 1. Check that the current phase can transition to Failed.
        //    (Only active phases can fail, not Pending, Completed, or Failed itself.)
        // 2. Set self.state.phase to Failed.
        // 3. Set self.state.error to Some(error.to_string()).
        // 4. Save a checkpoint.
        todo!("Transition to Failed state with error message")
    }

    /// Save the current state as a checkpoint.
    fn save_checkpoint(&self) -> Result<()> {
        // TODO:
        // 1. Serialize self.state to JSON using serde_json::to_string.
        // 2. Save to the store with key = "job:{job_id}".
        // 3. Map serialization errors to BatchError::CheckpointError.
        todo!("Serialize and save job state")
    }

    /// Load a checkpoint from the store and resume.
    ///
    /// Returns None if no checkpoint exists for this job.
    pub fn load_checkpoint(job_id: &str, store: MemoryStore) -> Result<Option<Self>> {
        // TODO:
        // 1. Load the value from store with key = "job:{job_id}".
        // 2. If None, return Ok(None).
        // 3. If Some, deserialize the JSON to JobState.
        // 4. Return a new BatchProcessor with the loaded state.
        todo!("Load and deserialize job checkpoint")
    }

    /// Determine which phase to resume from.
    ///
    /// Returns the next phase that should be started based on the current state.
    /// For completed intermediate phases (Prepared, Extracted, Embedded),
    /// returns the next active phase.
    /// For active phases (Preparing, Extracting, etc.), returns the same phase
    /// (it needs to be retried).
    pub fn resume_phase(&self) -> Result<Phase> {
        // TODO: Based on self.state.phase, return the phase to resume from.
        //
        // Prepared -> Extracting (preparation done, start extraction)
        // Extracted -> Embedding (extraction done, start embedding)
        // Embedded -> Storing (embedding done, start storing)
        // Preparing -> Preparing (retry the prepare phase)
        // Extracting -> Extracting (retry extraction)
        // Embedding -> Embedding (retry embedding)
        // Storing -> Storing (retry storing)
        // Pending -> Preparing (start from the beginning)
        // Completed -> error (already done)
        // Failed -> error (cannot resume failed jobs)
        todo!("Determine the resume phase")
    }
}

fn main() {
    println!("Batch State Machine Exercise");

    let store = MemoryStore::new();
    let mut processor = BatchProcessor::new("job-001", store);

    // Walk through the pipeline
    println!("Starting phase: {}", processor.state().phase);

    processor.advance_to(Phase::Preparing).unwrap();
    println!("Advanced to: {}", processor.state().phase);

    processor.advance_to(Phase::Prepared).unwrap();
    println!("Advanced to: {}", processor.state().phase);

    println!("\nAll transitions completed successfully!");
}
```
:::

::: hint level=1 title="Phase transition table"
Think of valid transitions as a lookup table:
```rust
matches!(
    (from, to),
    (Phase::Pending, Phase::Preparing)
    | (Phase::Preparing, Phase::Prepared)
    | (Phase::Preparing, Phase::Failed)
    | (Phase::Prepared, Phase::Extracting)
    // ... continue the pattern
)
```
:::

::: hint level=2 title="Saving and loading checkpoints"
```rust
fn save_checkpoint(&self) -> Result<()> {
    let json = serde_json::to_string(&self.state)
        .map_err(|e| BatchError::CheckpointError(e.to_string()))?;
    let key = format!("job:{}", self.state.job_id);
    self.store.save(&key, &json);
    Ok(())
}
```
:::

::: hint level=3 title="Resume phase logic"
```rust
pub fn resume_phase(&self) -> Result<Phase> {
    match &self.state.phase {
        Phase::Pending => Ok(Phase::Preparing),
        Phase::Preparing => Ok(Phase::Preparing),
        Phase::Prepared => Ok(Phase::Extracting),
        Phase::Extracting => Ok(Phase::Extracting),
        Phase::Extracted => Ok(Phase::Embedding),
        // ...
        Phase::Completed => Err(BatchError::TerminalState("completed".into())),
        Phase::Failed => Err(BatchError::TerminalState("failed".into())),
    }
}
```
:::

::: solution
```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Pending,
    Preparing,
    Prepared,
    Extracting,
    Extracted,
    Embedding,
    Embedded,
    Storing,
    Completed,
    Failed,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Phase::Pending => write!(f, "pending"),
            Phase::Preparing => write!(f, "preparing"),
            Phase::Prepared => write!(f, "prepared"),
            Phase::Extracting => write!(f, "extracting"),
            Phase::Extracted => write!(f, "extracted"),
            Phase::Embedding => write!(f, "embedding"),
            Phase::Embedded => write!(f, "embedded"),
            Phase::Storing => write!(f, "storing"),
            Phase::Completed => write!(f, "completed"),
            Phase::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: String,
    pub phase: Phase,
    pub total_documents: usize,
    pub processed_documents: usize,
    pub error: Option<String>,
    pub updated_at: String,
}

pub struct MemoryStore {
    data: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
        }
    }

    pub fn save(&self, key: &str, value: &str) {
        self.data.lock().unwrap().insert(key.to_string(), value.to_string());
    }

    pub fn load(&self, key: &str) -> Option<String> {
        self.data.lock().unwrap().get(key).cloned()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BatchError {
    #[error("Invalid phase transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Job is in a terminal state: {0}")]
    TerminalState(String),

    #[error("Checkpoint error: {0}")]
    CheckpointError(String),

    #[error("Processing error: {0}")]
    ProcessingError(String),
}

pub type Result<T> = std::result::Result<T, BatchError>;

pub fn is_valid_transition(from: &Phase, to: &Phase) -> bool {
    matches!(
        (from, to),
        (Phase::Pending, Phase::Preparing)
            | (Phase::Preparing, Phase::Prepared)
            | (Phase::Preparing, Phase::Failed)
            | (Phase::Prepared, Phase::Extracting)
            | (Phase::Extracting, Phase::Extracted)
            | (Phase::Extracting, Phase::Failed)
            | (Phase::Extracted, Phase::Embedding)
            | (Phase::Embedding, Phase::Embedded)
            | (Phase::Embedding, Phase::Failed)
            | (Phase::Embedded, Phase::Storing)
            | (Phase::Storing, Phase::Completed)
            | (Phase::Storing, Phase::Failed)
    )
}

pub struct BatchProcessor {
    store: MemoryStore,
    state: JobState,
}

impl BatchProcessor {
    pub fn new(job_id: &str, store: MemoryStore) -> Self {
        let state = JobState {
            job_id: job_id.to_string(),
            phase: Phase::Pending,
            total_documents: 0,
            processed_documents: 0,
            error: None,
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        Self { store, state }
    }

    pub fn state(&self) -> &JobState {
        &self.state
    }

    pub fn advance_to(&mut self, next_phase: Phase) -> Result<()> {
        if !is_valid_transition(&self.state.phase, &next_phase) {
            return Err(BatchError::InvalidTransition {
                from: self.state.phase.to_string(),
                to: next_phase.to_string(),
            });
        }

        self.state.phase = next_phase;
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        self.save_checkpoint()?;

        Ok(())
    }

    pub fn fail(&mut self, error: &str) -> Result<()> {
        if !is_valid_transition(&self.state.phase, &Phase::Failed) {
            return Err(BatchError::InvalidTransition {
                from: self.state.phase.to_string(),
                to: "failed".to_string(),
            });
        }

        self.state.phase = Phase::Failed;
        self.state.error = Some(error.to_string());
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        self.save_checkpoint()?;

        Ok(())
    }

    fn save_checkpoint(&self) -> Result<()> {
        let json = serde_json::to_string(&self.state)
            .map_err(|e| BatchError::CheckpointError(e.to_string()))?;
        let key = format!("job:{}", self.state.job_id);
        self.store.save(&key, &json);
        Ok(())
    }

    pub fn load_checkpoint(job_id: &str, store: MemoryStore) -> Result<Option<Self>> {
        let key = format!("job:{}", job_id);
        match store.load(&key) {
            None => Ok(None),
            Some(json) => {
                let state: JobState = serde_json::from_str(&json)
                    .map_err(|e| BatchError::CheckpointError(e.to_string()))?;
                Ok(Some(Self { store, state }))
            }
        }
    }

    pub fn resume_phase(&self) -> Result<Phase> {
        match &self.state.phase {
            Phase::Pending => Ok(Phase::Preparing),
            Phase::Preparing => Ok(Phase::Preparing),
            Phase::Prepared => Ok(Phase::Extracting),
            Phase::Extracting => Ok(Phase::Extracting),
            Phase::Extracted => Ok(Phase::Embedding),
            Phase::Embedding => Ok(Phase::Embedding),
            Phase::Embedded => Ok(Phase::Storing),
            Phase::Storing => Ok(Phase::Storing),
            Phase::Completed => Err(BatchError::TerminalState("completed".to_string())),
            Phase::Failed => Err(BatchError::TerminalState("failed".to_string())),
        }
    }
}

fn main() {
    println!("Batch State Machine Exercise");

    let store = MemoryStore::new();
    let mut processor = BatchProcessor::new("job-001", store);

    println!("Starting phase: {}", processor.state().phase);

    processor.advance_to(Phase::Preparing).unwrap();
    println!("Advanced to: {}", processor.state().phase);

    processor.advance_to(Phase::Prepared).unwrap();
    println!("Advanced to: {}", processor.state().phase);

    processor.advance_to(Phase::Extracting).unwrap();
    processor.advance_to(Phase::Extracted).unwrap();
    processor.advance_to(Phase::Embedding).unwrap();
    processor.advance_to(Phase::Embedded).unwrap();
    processor.advance_to(Phase::Storing).unwrap();
    processor.advance_to(Phase::Completed).unwrap();

    println!("Final phase: {}", processor.state().phase);
    println!("\nAll transitions completed successfully!");
}
```

### Explanation

The state machine enforces a strict pipeline order:

- **Transition validation**: `is_valid_transition` uses pattern matching to enumerate
  all legal transitions. This prevents bugs like jumping from Pending directly to
  Storing, which would skip extraction and embedding.

- **Active vs completed phases**: Each pipeline stage has two states (e.g., `Preparing`
  and `Prepared`). The active state means work is in progress; the completed state
  means the checkpoint is safe. This distinction is critical for resume: if the system
  crashes during `Preparing`, that phase is retried; if it crashes after `Prepared`,
  the next phase starts.

- **Checkpoints**: Every transition saves the job state to the store. In production,
  this is DynamoDB. The checkpoint includes the current phase, progress counters,
  and timestamps.

- **Resume logic**: `resume_phase()` determines the correct phase to start based on
  the last checkpoint. Completed intermediate phases advance to the next active phase.
  Active phases are retried. Terminal states (Completed, Failed) return errors.

- **Failure handling**: Only active phases (Preparing, Extracting, Embedding, Storing)
  can transition to Failed. The error message is preserved for debugging.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- Transition validation tests ---

    #[test]
    fn test_valid_forward_transitions() {
        assert!(is_valid_transition(&Phase::Pending, &Phase::Preparing));
        assert!(is_valid_transition(&Phase::Preparing, &Phase::Prepared));
        assert!(is_valid_transition(&Phase::Prepared, &Phase::Extracting));
        assert!(is_valid_transition(&Phase::Extracting, &Phase::Extracted));
        assert!(is_valid_transition(&Phase::Extracted, &Phase::Embedding));
        assert!(is_valid_transition(&Phase::Embedding, &Phase::Embedded));
        assert!(is_valid_transition(&Phase::Embedded, &Phase::Storing));
        assert!(is_valid_transition(&Phase::Storing, &Phase::Completed));
    }

    #[test]
    fn test_valid_failure_transitions() {
        assert!(is_valid_transition(&Phase::Preparing, &Phase::Failed));
        assert!(is_valid_transition(&Phase::Extracting, &Phase::Failed));
        assert!(is_valid_transition(&Phase::Embedding, &Phase::Failed));
        assert!(is_valid_transition(&Phase::Storing, &Phase::Failed));
    }

    #[test]
    fn test_invalid_skip_transitions() {
        assert!(!is_valid_transition(&Phase::Pending, &Phase::Extracting));
        assert!(!is_valid_transition(&Phase::Pending, &Phase::Completed));
        assert!(!is_valid_transition(&Phase::Prepared, &Phase::Embedded));
        assert!(!is_valid_transition(&Phase::Preparing, &Phase::Storing));
    }

    #[test]
    fn test_invalid_backward_transitions() {
        assert!(!is_valid_transition(&Phase::Prepared, &Phase::Pending));
        assert!(!is_valid_transition(&Phase::Completed, &Phase::Pending));
        assert!(!is_valid_transition(&Phase::Extracted, &Phase::Preparing));
    }

    #[test]
    fn test_terminal_states_have_no_transitions() {
        assert!(!is_valid_transition(&Phase::Completed, &Phase::Preparing));
        assert!(!is_valid_transition(&Phase::Completed, &Phase::Failed));
        assert!(!is_valid_transition(&Phase::Failed, &Phase::Preparing));
        assert!(!is_valid_transition(&Phase::Failed, &Phase::Pending));
    }

    #[test]
    fn test_non_active_phases_cannot_fail() {
        assert!(!is_valid_transition(&Phase::Pending, &Phase::Failed));
        assert!(!is_valid_transition(&Phase::Prepared, &Phase::Failed));
        assert!(!is_valid_transition(&Phase::Extracted, &Phase::Failed));
        assert!(!is_valid_transition(&Phase::Embedded, &Phase::Failed));
    }

    // --- BatchProcessor tests ---

    #[test]
    fn test_full_pipeline() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-1", store);

        assert_eq!(proc.state().phase, Phase::Pending);

        proc.advance_to(Phase::Preparing).unwrap();
        proc.advance_to(Phase::Prepared).unwrap();
        proc.advance_to(Phase::Extracting).unwrap();
        proc.advance_to(Phase::Extracted).unwrap();
        proc.advance_to(Phase::Embedding).unwrap();
        proc.advance_to(Phase::Embedded).unwrap();
        proc.advance_to(Phase::Storing).unwrap();
        proc.advance_to(Phase::Completed).unwrap();

        assert_eq!(proc.state().phase, Phase::Completed);
    }

    #[test]
    fn test_invalid_advance_rejected() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-2", store);

        let result = proc.advance_to(Phase::Extracting);
        assert!(result.is_err());
        assert_eq!(proc.state().phase, Phase::Pending);
    }

    #[test]
    fn test_fail_during_processing() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-3", store);

        proc.advance_to(Phase::Preparing).unwrap();
        proc.fail("Out of memory").unwrap();

        assert_eq!(proc.state().phase, Phase::Failed);
        assert_eq!(proc.state().error, Some("Out of memory".to_string()));
    }

    #[test]
    fn test_cannot_advance_after_failure() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-4", store);

        proc.advance_to(Phase::Preparing).unwrap();
        proc.fail("Error").unwrap();

        let result = proc.advance_to(Phase::Prepared);
        assert!(result.is_err());
    }

    #[test]
    fn test_checkpoint_and_resume() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-5", store);

        // Advance to Extracted
        proc.advance_to(Phase::Preparing).unwrap();
        proc.advance_to(Phase::Prepared).unwrap();
        proc.advance_to(Phase::Extracting).unwrap();
        proc.advance_to(Phase::Extracted).unwrap();

        // Simulate crash: create new processor from checkpoint
        let store2 = MemoryStore::new();
        // Copy the checkpoint data
        let key = "job:test-5";
        let checkpoint = proc.store.load(key).unwrap();
        store2.save(key, &checkpoint);

        let resumed = BatchProcessor::load_checkpoint("test-5", store2)
            .unwrap()
            .expect("Checkpoint should exist");

        assert_eq!(resumed.state().phase, Phase::Extracted);
        assert_eq!(resumed.resume_phase().unwrap(), Phase::Embedding);
    }

    #[test]
    fn test_resume_from_active_phase() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-6", store);

        proc.advance_to(Phase::Preparing).unwrap();
        // Simulate crash during Preparing (before Prepared checkpoint)

        let store2 = MemoryStore::new();
        let key = "job:test-6";
        let checkpoint = proc.store.load(key).unwrap();
        store2.save(key, &checkpoint);

        let resumed = BatchProcessor::load_checkpoint("test-6", store2)
            .unwrap()
            .expect("Checkpoint should exist");

        // Should retry the Preparing phase
        assert_eq!(resumed.resume_phase().unwrap(), Phase::Preparing);
    }

    #[test]
    fn test_resume_completed_is_error() {
        let store = MemoryStore::new();
        let mut proc = BatchProcessor::new("test-7", store);

        proc.advance_to(Phase::Preparing).unwrap();
        proc.advance_to(Phase::Prepared).unwrap();
        proc.advance_to(Phase::Extracting).unwrap();
        proc.advance_to(Phase::Extracted).unwrap();
        proc.advance_to(Phase::Embedding).unwrap();
        proc.advance_to(Phase::Embedded).unwrap();
        proc.advance_to(Phase::Storing).unwrap();
        proc.advance_to(Phase::Completed).unwrap();

        let result = proc.resume_phase();
        assert!(result.is_err());
    }

    #[test]
    fn test_load_missing_checkpoint() {
        let store = MemoryStore::new();
        let result = BatchProcessor::load_checkpoint("nonexistent", store).unwrap();
        assert!(result.is_none());
    }
}
```
:::

::: reflection
- In the real EdgeQuake batch pipeline, what happens if the Extract phase partially
  completes (50 of 100 documents extracted) before crashing? How would you make
  resume skip the already-extracted documents?
- Could you extend this state machine to support parallel phases (e.g., embedding
  different document batches concurrently)? What changes would be needed?
- Why is `Failed` a terminal state? What would a production system need to support
  retrying failed jobs?
:::
