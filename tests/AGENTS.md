# Test Execution Guidance

- Keep test runtime fast. Prefer short timeout values in tests (for example, hundreds of milliseconds, not multi-second waits) unless the scenario explicitly requires longer timing windows.
- Do not "fix" flaky timing tests by broadly increasing timeouts. First prefer deterministic harness behavior:
  - Exit wait loops as soon as required signals/acks are observed.
  - Use bounded polling with small sleep intervals.
  - Stop sessions promptly once success criteria are met.
- For reconnect/socket tests, keep reconnect and ack deadlines short, and structure the harness so each session can complete immediately after expected acks are received.
- If timing instability appears, make the test synchronization logic more explicit before increasing any timeout values.
