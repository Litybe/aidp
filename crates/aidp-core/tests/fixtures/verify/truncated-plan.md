# Plan (truncated)

We will refactor the payload validator so it rejects malformed events before
they reach the handler, then extend the publisher to emit the new
`validation_failed_v1` event on rejection.

```rust
pub fn validate(payload: &Payload) -> Result<(), ValidationError> {
    if payload.user_id.is_empty() {
        return Err(ValidationError::MissingUserId);
    }
    // the file was cut off here — this fence is intentionally never closed,
    // to exercise verify::verify_outputs's unbalanced-fence check.
