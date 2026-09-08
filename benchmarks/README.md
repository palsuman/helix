# Platform Performance Suite

Run from the repository root with the pinned Rust toolchain and Node 24.15.0:

```sh
npm run bench:test
npm run bench
npm run bench:check
npm run bench:workspace
```

No JavaScript package installation or Tauri build is required. Criterion and
platform memory APIs are exactly pinned in the benchmark-only `helix-bench`
workspace crate. `cargo bench -p helix-bench --bench platform -- --test` smoke-tests
the benchmark operations without collecting a baseline.

## Workloads

| Case                | Measured operation                                                                                                                                                                                                                               |
| ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `container_startup` | Start five dependency-ordered singleton services in the real service container. Factories have minimal lifecycle bodies, isolating container overhead. Registration and shutdown are outside timing. This is not full kernel bootstrap.          |
| `ipc_round_trip`    | Request serialization, authenticated loopback TCP RPC, real ping dispatch, response serialization/deserialization, and result validation. The listener and runtime are prestarted. This does not include a native WebView/Tauri invoke boundary. |
| `file_read`         | Read and decode a warm-cache 65,520-byte UTF-8 source file through `FileSystemService`, including hashing and correctness validation.                                                                                                            |
| `file_write`        | Overwrite the same source through the real atomic-write service, including encoding, hashing, fsync, and rename. Post-run contents are verified.                                                                                                 |
| `config_merge`      | Deep merge 200 nested sections, including scalar overrides and array replacement. Each input clone is prepared outside timing with per-iteration batching; output consumption/destruction and a representative assertion are included.           |

Each case has 30 samples, a one-second warmup, and a three-second target
measurement period. Timing includes explicitly described validation and harness
overhead. Use these as regression microbenchmarks, not absolute product SLA
measurements. The write benchmark reflects the filesystem used for OS temporary
directories; keep disk, power policy, and temporary-directory configuration fixed.

## Peak Memory

The runner launches one case per fresh benchmark process. After Criterion
finishes, that process records its OS high-water RSS in bytes: `getrusage` on
Linux/macOS and peak working set on Windows. The number includes Criterion,
runtime, fixtures, warmup, and measurements, not just allocations inside the
operation. It is not an allocation count or a before/after delta. Separate
processes prevent a previous case's high-water mark contaminating another case.

## Reports and Gate

`target/performance/latest.json` contains mean nanoseconds, confidence bounds,
peak RSS, revision, dirty-worktree flag, and runner metadata. Each uniquely named
run directory retains Criterion estimates, samples, RSS reports, and stdout/stderr.
Failed or timed-out operations cannot reuse old results. Override the report path
with `npm run bench -- path/to/report.json`.

`bench:check` compares against the checked-in `baseline.json` and exits nonzero
if **either mean latency or peak RSS increases by more than 10%** for any case.
Exactly 10% passes. Missing cases, nonpositive/nonfinite metrics, incompatible
protocols, and mismatched runner metadata also fail. The gate uses point estimates;
confidence bounds are retained for diagnosis, not used to waive regressions.
It writes `target/performance/comparison.json` and prints every delta.

Measurements are sensitive to load, thermal throttling, filesystem behavior,
and OS changes. Investigate failures and repeat on an idle reference machine;
never silently widen the threshold or accept a slow baseline to turn CI green.

## Reference Monorepo

The generator creates exactly **50,000 files**: 100 package manifests, 49,897
TypeScript sources, a root npm-workspace manifest, Nx configuration, and the
generator manifest. Package dependencies form a deterministic chain. There are
no downloads, timestamps, random contents, or symlinks. A nonempty destination
is refused, including a previously generated workspace.

```sh
npm run bench:workspace -- target/my-reference-workspace
```

This dataset is for subsequent search/index/explorer benchmarks. The current
microbenchmarks intentionally use fixed small fixtures and do not claim to scan
the 50k-file dataset. Generator tests verify deterministic contents, exact file
count, dependencies, and non-destructive behavior.

## CI Provisioning

The workflow runs tooling tests on hosted Linux for PRs and pushes. Measurements
and the regression gate run on trusted main/master pushes or manual dispatch,
on a **dedicated self-hosted runner labeled `helix-performance`**. They never run
untrusted PR code on that runner. Runs are serialized, and raw results plus
comparison reports are uploaded for 90 days, keyed by revision and run attempt.

The initial measured bootstrap baseline is Apple M5, 10 logical CPUs, macOS
Darwin 25.6.0, arm64, Rust 1.97.1, runner ID `reference-v1`. It was measured from
the implementation worktree, which is recorded as dirty. It is not a measurement
from a GitHub-hosted runner. The project owner must provision the dedicated runner
and calibrate a release baseline there before enabling this as a required CI gate.
If no runner has that label, the measurement job queues; it does not establish a
passing CI result. Do not compare this baseline against arbitrary hosted hardware.

Runner identity includes OS release, architecture, CPU model/count, Rust version,
and `HELIX_BENCH_RUNNER` (default `reference-v1`). Changes to these require an
explicit baseline review. Memory APIs are provided for Linux/macOS/Windows, but
only macOS measurements have been validated locally.

## Release Baseline Updates

1. At a release, check out the intended release commit on the idle dedicated
   reference runner. Confirm power/disk configuration and pinned toolchain.
2. Run `npm run bench:test`, then collect at least three independent runs using
   distinct report paths. Review raw samples and confidence intervals. Investigate
   unexplained differences; do not cherry-pick the slowest run.
3. Select a representative reviewed report, record the reason and release, then:

   ```sh
   npm run bench:update -- --report target/performance/release.json --release 0.2.0 --reason "Reviewed release baseline on reference-v1"
   ```

4. Review the JSON diff and retain the run artifacts in the release record. Commit
   the baseline with the release changes. CI never promotes a baseline automatically.
5. Run a fresh measurement and `bench:check` on the reference runner to verify the
   gate. Hardware or toolchain migrations require this same release review.

Change the protocol identifier when changing fixtures, timing semantics, sample
configuration, or the Criterion version. Do not compare different protocols.
