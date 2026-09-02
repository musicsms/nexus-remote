wrote /home/ubuntu/nexus-remote/.worktrees/windows-platform-backends/.superpowers/sdd/2026-09-02-native-windows-client/task-5-brief.md: 74 lines
### Task 5: Wire Client Binary and Synthetic Loopback Integration

**Files:**

- Modify: `apps/nexus-client/src/main.rs`
- Modify: `apps/nexus-client/src/lib.rs`
- Create: `apps/nexus-client/tests/client_loopback_e2e.rs`
- Modify: `docs/IMPLEMENTATION_STATUS.md`
- Modify: `README.md`

**Interfaces:**
- Produces `ClientRuntime::connect`, `run`, and bounded `shutdown` orchestration.
- Consumes session, receiver, renderer, window, and input modules without exposing private Windows handles.

- [ ] **Step 1: Write failing loopback test**

Use existing QUIC loopback helpers and a deterministic key to send one sealed `VideoPacketHeader`/payload and one semantic input control message. Assert the client verifies/open/reassembles exactly one frame job and emits exactly one validated input message.

- [ ] **Step 2: Run loopback test and observe RED**

Run: `cargo test -p nexus-client --test client_loopback_e2e`

Expected: compile failure because `ClientRuntime` and integration wiring do not exist.

- [ ] **Step 3: Implement runtime orchestration**

Wire bounded Tokio network tasks to the session and receiver; hand jobs to the depth-one renderer queue and window thread. Treat `OutputPending`/frame-unavailable as non-fatal, propagate authentication/expiry/device errors, and preserve session ID on reconnect.

- [ ] **Step 4: Run loopback test and observe GREEN**

Run: `cargo test -p nexus-client --test client_loopback_e2e`

Expected: one authenticated frame job and one semantic input message with no plaintext logging.

- [ ] **Step 5: Replace the stub main**

Initialize tracing, load validated client configuration, call `ClientRuntime`, and return explicit user-visible errors. Do not add unattended private-key handling or browser dependencies.

- [ ] **Step 6: Synchronize status docs**

Mark `nexus-client` **In progress** with the exact implemented modules and test evidence. Keep Phase 1 **In progress** and record absent MSVC/live-Windows smoke/full host-service-relay acceptance.

- [ ] **Step 7: Run complete verification**

```bash
cargo fmt --all -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo check -p nexus-client --tests --target x86_64-pc-windows-gnu
cargo clippy -p nexus-client --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

Expected: all available checks exit zero; Windows-only smoke tests remain explicitly ignored on Linux.

- [ ] **Step 8: Commit**

```bash
git add apps/nexus-client docs/IMPLEMENTATION_STATUS.md README.md
git commit -m "feat(client): wire native viewer loopback runtime"
```

## Final Verification Checklist

- [ ] `nexus-client` is no longer a stub.
- [ ] Capability and relay token are verified before transport.
- [ ] Malformed/oversized/replayed/tampered video packets are rejected.
- [ ] Frame AEAD AAD/nonce matches encoded frame metadata.
- [ ] Depth-one render queue drops stale frames.
- [ ] Win32 window, D3D11 renderer, and Media Foundation decoder are isolated on native threads.
- [ ] Semantic input/cursor validation and focus gating are covered.
- [ ] Synthetic QUIC loopback proves authenticated receive/render handoff and input emission.
- [ ] Linux workspace checks and GNU Windows target checks pass.
- [ ] Windows interactive smoke results are not claimed without actually running them.
- [ ] Phase 1 remains In progress until full host/client/service/relay acceptance and measurements pass.
