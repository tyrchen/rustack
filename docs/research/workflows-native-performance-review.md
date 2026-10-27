# Native-host Workflows Performance Review

Status: measured current-host evidence; required Linux reference and image/platform gates remain open · Decision: [D74](../../specs/rustack-workflows-99-decisions.md#d74--measure-pure-workflow-latency-and-loop-responsiveness-through-the-real-binary)

`make test-workflows-performance` completed after the broader verification jobs had joined. It executes six normal measurement/causality controls, both real signed SDK workloads and denied-warning/pedantic Clippy. Source and tested binary fingerprints below identify this capture. The parent commit identifies the production baseline; the source fingerprints identify the new harness before its implementation commit. No production workflow code changes in this measurement slice. The retained unfinished Docker checkpoint was present in the worktree but was not exercised; the binary fingerprint identifies the tested artifact rather than claiming a clean-HEAD build.

The actual server is a local macOS/aarch64 debug-assertion build with18 available logical CPUs. The workloads execute no Task states; joined process cleanup proves zero Native Lambda processes. CPU availability does not prove dedicated capacity, memory size or the specified4CPU/8GiB Linux reference. The measurements include SDK signing, local HTTP and terminal polling. They are a successful current-host check, with `referenceQualification=pending`.

| Measurement | Observed (µs) | Contract |
|---|---:|---|
|100×50-state/1KiB, p50 |10346 | Report all100 ordered samples |
|100×50-state/1KiB, p95 |10508 |≤100000 |
|100×50-state/1KiB, maximum |10644 | Each observation<2000000 |
|100 live-loop cycles, readiness maximum |283 |<2000000 |
|100 live-loop cycles, Stop→ABORTED maximum |1102 |<2000000 |
|History-exhaustion readiness maximum |348 |<2000000, at least one live bracket |
|History-exhaustion live/all probes |137/137 |RUNNING→probe→RUNNING |
|History-exhaustion final event ID |7560 | Reserved ExecutionFailed,≤25000 |

Every latency execution preserves exact input/output and102 sequential typed history events. The loop reaches its configured history byte/event bound with States.HistoryLimitExceeded, then100 immediate one-slot starts each observe live work and finish ABORTED with the requested stop error. Configured accounting limits do not claim process RSS bounds. No Docker, VM, image latency, image log-flood, container cleanup, IaC or platform acceptance is inferred.

The captured harness is retained in commit `43054d3`; recorded source hashes refer to that historical capture, before later fixture evolution.

## Capture provenance

```json
{
  "baseCommit": "a9b9f60d2a24537a0afbf4e4738e43936de0b0c6",
  "decision": "D74",
  "capturedAt": "2026-10-27T06:59:01.493816+00:00",
  "rustc": "rustc 1.99.1 (58cf8f9c3 2026-10-13)",
  "binarySha256": "e9fa58ece6ebe3f1cb5d2087fed849b1d597320f52d1a3144cd4415bde0e3a0a",
  "sourceSha256": {
    "Cargo.lock": "f88b8c7b4227d2bfe6ec8fa45570cebbafa01b309932402fb7ace07fe8cda34a",
    "apps/rustack/tests/workflows_sdk.rs": "e1f4aee8ee16b4e397af00dc48c28bf6fe01106c1eb42ba37adf82cc75169f90",
    "apps/rustack/tests/workflows_sdk/performance.rs": "7b3ab9eadfafdc1d1d37349eeb8ca3ebcb4c38ca6402820318f735ba14c4f4d5",
    "apps/rustack/tests/workflows_sdk/performance/latency.rs": "8f68ae3ed3753642153dc23c0fd0f501e72879bd9bc0b43498367e01b6ee193f",
    "apps/rustack/tests/workflows_sdk/performance/load.rs": "34f5f9c26d686d98e45a737bc652c742115093d8fb05d15b82e6694c30d7dc0e",
    "apps/rustack/tests/workflows_sdk/performance/tests.rs": "313ce95fc0adf1b5d79704557530ff4d208438ca67801b188c86494f1bdf03ea",
    "apps/rustack/tests/workflows_sdk/process.rs": "2750e27807c160ee0f6cd1567b946e6a6b20d7b049c22522ae8acd7b0ebd1c03",
    "apps/rustack/tests/workflows_sdk/runner.rs": "d3b9b31be370fd0fff26ace796ce2e87d3c463901627fe6613ff2595abda6f86",
    "apps/rustack/tests/workflows_sdk/rust.rs": "0c87f01f98c5f4ad329dc0542a57221e4c387d5de43df523166959a187996338"
  }
}
```

## Raw reports

Each JSON line is the original bounded report emitted only after server cleanup. Raw sample order is preserved; summaries use nearest ranks50/95/100 and round durations up to whole microseconds.

```json
{"schemaVersion":1,"host":{"os":"macos","architecture":"aarch64","availableLogicalCpus":18,"debugAssertions":true,"referenceQualification":"pending"},"limits":{"openExecutions":1,"transitions":1000000,"historyEvents":25000,"historyBytes":33554432,"totalHistoryBytes":134217728},"nativeProcessesObserved":0,"serverJoined":true,"measurement":{"kind":"loopResponsiveness","openExecutionSlots":1,"completedCycles":100,"inputBytes":1024,"historyEventHighWater":7560,"exhaustion":{"probeCount":137,"liveProbeCount":137,"maxReadinessMicros":348},"readiness":{"count":100,"observedMicros":[283,274,274,282,267,259,257,265,242,240,229,225,226,250,232,207,219,224,218,235,223,229,213,219,219,216,222,230,225,221,225,224,219,220,224,223,220,218,221,217,216,207,217,219,226,220,237,225,219,221,224,230,222,224,223,253,216,249,217,226,216,220,223,225,219,225,225,224,217,227,221,226,216,229,215,218,216,217,222,222,219,215,223,220,216,219,224,218,217,224,220,220,209,227,217,214,218,219,233,224],"p50Micros":222,"p95Micros":265,"maxMicros":283},"stop":{"count":100,"observedMicros":[1058,1023,1027,1032,1068,1001,1027,1027,1016,973,1001,1000,1021,1102,1048,991,1022,1012,1012,1038,1016,1030,976,974,1019,1012,1000,998,1035,1023,958,1064,1018,1001,1035,984,1032,997,971,1032,1010,983,986,1034,1062,1039,1010,996,1028,988,1001,1031,977,989,1008,1012,982,1005,975,1024,1015,966,1024,1025,1028,979,1024,1057,1038,1055,999,980,1015,1005,1076,1052,1079,1025,1050,1033,969,1053,1037,1035,1037,1068,1046,1018,1028,1041,1043,1024,1013,1061,1000,1041,1035,1090,1051,1035],"p50Micros":1023,"p95Micros":1068,"maxMicros":1102}}}
{"schemaVersion":1,"host":{"os":"macos","architecture":"aarch64","availableLogicalCpus":18,"debugAssertions":true,"referenceQualification":"pending"},"limits":{"openExecutions":1,"transitions":1000000,"historyEvents":25000,"historyBytes":33554432,"totalHistoryBytes":134217728},"nativeProcessesObserved":0,"serverJoined":true,"measurement":{"kind":"pureLatency","stateCount":50,"inputBytes":1024,"executions":100,"budgetMicros":100000,"latency":{"count":100,"observedMicros":[10644,10172,10264,10177,10080,10187,10263,10308,9812,10367,10388,7594,10353,10258,10274,10325,10293,10350,10441,10404,9943,10279,10338,10363,10337,10398,10235,10390,10397,10352,10411,10298,10334,10371,10340,10276,10311,10391,10333,10102,10357,10335,10386,10363,10266,10276,8990,10302,10323,10312,10228,10261,10345,10299,10390,10220,10337,10381,10338,10209,10369,10308,10284,10385,10359,10309,10290,10415,10278,10376,9835,10377,10371,10351,10388,10353,10420,10394,10346,10287,10416,10447,10389,10017,10471,10450,10561,10555,10473,10508,10384,10512,10477,10519,10484,10045,10352,10425,10488,10484],"p50Micros":10346,"p95Micros":10508,"maxMicros":10644}}}
```

See [verification§5.1](../../specs/rustack-workflows-70-verification-plan.md#51-native-host-pure-workflow-workload-and-responsiveness) for the executable contract and [review W211–W213](../../specs/rustack-workflows-93-review.md#p5-pure-workload-and-responsiveness-independent-review) for repaired evidence gaps.
