# Automatic Retry for Token Limit Errors

## Overview

The batch ingestion pipeline now automatically retries when hitting OpenAI's 2M token limit, making it truly hands-off for long-running jobs (hours to days).

## How It Works

When the pipeline encounters a `token_limit_exceeded` error:

1. **Detects the error** - Identifies token limit errors specifically
2. **Waits intelligently** - Uses exponential backoff (60s → 120s → 240s → ...)
3. **Retries automatically** - Up to 10 attempts by default
4. **Caps maximum wait** - Never waits more than 30 minutes between retries
5. **Logs progress** - Shows countdown and attempt numbers

## Default Behavior

```bash
make batch-run
```

- **Max retries**: 10 attempts
- **Initial delay**: 60 seconds
- **Backoff strategy**: Exponential (doubles each time)
- **Max delay**: 30 minutes (1800 seconds)
- **Total max wait**: ~34 hours (if all 10 retries hit max delay)

## Configuration

### Via Command Line

```bash
# Custom retry settings
cargo run -p edgequake-batch -- \
  --data data.parquet \
  --api-key $OPENAI_API_KEY \
  --max-retries 20 \
  --retry-delay 120 \
  run
```

### Via Environment Variables

```bash
export BATCH_MAX_RETRIES=20
export BATCH_RETRY_DELAY=120
make batch-run
```

### Via Makefile

Add to your `.env` file:
```
BATCH_MAX_RETRIES=20
BATCH_RETRY_DELAY=120
```

## Example Output

```
INFO Phase 2: EXTRACT starting
INFO 💡 Automatic retry enabled: Will retry up to 10 times if token limits are hit (exponential backoff: 60s → 120s → ...)
INFO Processing batch file file=1 total=16
WARN Token limit exceeded - waiting for in-progress batches to complete attempt=1 max_attempts=10 retry_in_secs=60
INFO ⏳ Automatic retry 1/10 - sleeping for 60 seconds...
INFO 🔄 Retrying batch creation (attempt 2/10)
INFO Batch job created batch_id=batch_xxx
```

## Monitoring

### Check What's Queued

```bash
make batch-list
```

This shows:
- All batches in your OpenAI organization
- Which ones are in progress (consuming your token limit)
- Progress percentages
- When they were created/completed

### Example Output

```
=== OpenAI Batch Jobs (showing 26 most recent) ===

Batch ID:     batch_xxx
Status:       🔄 In Progress
Progress:     245/333 (73.6%)
Created:      2026-02-13 15:40:52 UTC

=== Summary ===
Total batches shown:     26
In progress/finalizing:  3
Pending requests:        156
💡 Tip: These pending requests are consuming your 2M token limit.
```

## When It Gives Up

After exhausting all retry attempts (default: 10), you'll see:

```
Error: Token limit exceeded after 10 retry attempts.
In-progress batches are still consuming the 2M token limit.
Use 'make batch-list' to check status. Wait longer or reduce batch size.
```

**Solutions:**
1. Run `make batch-list` to see what's queued
2. Wait for in-progress batches to complete
3. Increase `--max-retries` if needed
4. Reduce batch size with `--limit 500`

## Tuning Recommendations

### For Large Datasets (many batches)

```bash
# Be very patient
export BATCH_MAX_RETRIES=20      # More retries
export BATCH_RETRY_DELAY=120     # Longer initial wait
```

### For Small Datasets (few batches)

```bash
# Fail faster
export BATCH_MAX_RETRIES=5       # Fewer retries
export BATCH_RETRY_DELAY=30      # Shorter initial wait
```

### For Overnight Runs

```bash
# Set and forget
export BATCH_MAX_RETRIES=50      # Very patient
export BATCH_RETRY_DELAY=300     # 5 minute initial wait
```

## Technical Details

### What Errors Are Retried?

Only token limit errors:
- Error code: `token_limit_exceeded`
- Error message contains: `Enqueued token limit reached`

All other errors (API errors, network errors, etc.) fail immediately.

### Why Exponential Backoff?

1. **Respectful** - Doesn't hammer OpenAI's API
2. **Efficient** - Short waits for quick recoveries
3. **Patient** - Long waits when system is heavily loaded
4. **Capped** - Won't wait forever (30 min max)

### State Preservation

The retry mechanism:
- ✅ Saves state before each retry
- ✅ Preserves uploaded file IDs
- ✅ Can be interrupted (Ctrl+C) and resumed
- ✅ Logs all attempts for debugging

## Troubleshooting

### "Still hitting token limits after 10 retries"

**Cause**: Your organization has many long-running batches.

**Solution**:
```bash
# Check what's running
make batch-list

# Increase patience
export BATCH_MAX_RETRIES=30
make batch-resume
```

### "Want faster retries for testing"

**Cause**: Default 60s wait is too long for dev/testing.

**Solution**:
```bash
export BATCH_MAX_RETRIES=3
export BATCH_RETRY_DELAY=10  # 10s for quick testing
make batch-run
```

### "Want to disable retries completely"

**Cause**: Want immediate failure on token limits.

**Solution**:
```bash
export BATCH_MAX_RETRIES=1  # No retries
make batch-run
```

## See Also

- `make batch-list` - List all OpenAI batches
- `make batch-status` - Show local job status
- `make batch-resume` - Resume from checkpoint
