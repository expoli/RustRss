# T7 AC5 — native 200-row open and star performance

Final candidate source commit: `90f4275` (including the settings navigation spacing fix). Baseline source: `bc680d5155b7c6afcee355fe6aa9bc74ef456031`. The binary SHA-256 values, checked before both runs and in every raw result, are baseline `6a2ed07a8479c913457aa782bb8084bc8e02a4e6994e5c5e12e3756d22408284` and final candidate `4150b083de72077a4eda3706f16e9515f7a19b6f7193064856e5b3683031a353`. This extends the earlier [open-only evidence](../performance/record.md) with the star path on the final binary.

Both builds ran sequentially on the same Linux x86_64 host (16 logical CPUs), WebKitGTK 2.52.6, and private Xvfb 1400×1000 display. Each got an isolated HOME, data directory, runtime directory, inspector port, and byte-identical copy of [fixture-200.sqlite](fixture-200.sqlite), SHA-256 `cec0856c7d16785764b7542e0f767484a91a8f118e3fc90ec859603bb2dc897b`. The fixture renders 200 synthetic rows in All view; selected rows were already read and image free. No user database, live feed, Android device, or main checkout was modified.

## Measured result

One **untimed** open-and-star cycle warmed the code, fonts and SQLite cache in each fresh app process. Then 20 distinct rows each received an article open followed by a star toggle. All 20 **hot** cycles are in each raw JSON. `performance.now()` in native WebKitGTK measured (1) row click to matching reader heading and active row, (2) star button click to changed row marker, reader button label, and starred sidebar count after IPC, and (3) the full open-and-star pair. Inspector transport and Python polling were outside the timed intervals. P95 is nearest rank, the 19th sorted sample of 20.

| Paired run | Path | Baseline p95 | Final p95 | Change |
| --- | --- | ---: | ---: | ---: |
| A | Full open + star pair | 45 ms | 26 ms | −19 ms (−42.22%) |
| A | Open | 13 ms | 9 ms | −4 ms (−30.77%) |
| A | Star through count update | 24 ms | 17 ms | −7 ms (−29.17%) |
| B | Full open + star pair | 23 ms | 26 ms | +3 ms (+13.04%) |
| B | Open | 9 ms | 10 ms | +1 ms (+11.11%) |
| B | Star through count update | 15 ms | 17 ms | +2 ms (+13.33%) |

The approved gate calls for repair or rollback only when candidate p95 is **more than 20% above baseline and at least 16 ms higher**. Pair, open, and star each passed this gate in both runs. Run A's baseline was notably slower than run B's, so both complete runs are retained; these results show local acceptance under this policy, not a stable performance distribution. Fresh process startup itself was not timed.

## DOM behavior

The candidate retained identity of all 200 list row nodes through every measured cycle. Exactly 21 list child mutations occurred per build: one expected `<span class="star">` add or remove for the warmup and each of 20 measured toggles. There were no other list child mutations or full list rebuilds. In each candidate run, the sidebar had exactly 21 mutations, all the expected starred-count changes; unchanged view, feed and tag counts had no mutations. Hooks on `setAttribute`, `textContent`, and `innerHTML`, plus attribute/text mutation observation, recorded zero same-value DOM writes during candidate star toggles. A repeat render of the current All view with unchanged counts also recorded zero writes across the observed sidebar and count roots. The baseline produced 399 sidebar mutations per run during the same star path, including same-value attributes; this is comparative context, not a candidate failure.

## Raw artifacts and reproduction

- [run-a-summary.json](run-a-summary.json), [run-a-baseline-results.json](run-a-baseline-results.json), [run-a-candidate-results.json](run-a-candidate-results.json): 20 per-cycle timings, binary/fixture hashes, DOM node identity, mutation records, same-value write records, and gates.
- [run-b-summary.json](run-b-summary.json), [run-b-baseline-results.json](run-b-baseline-results.json), [run-b-candidate-results.json](run-b-candidate-results.json): independent second paired run with the same artifacts and workload.
- The matching `run-{a,b}-{baseline,candidate}-app.log` and `-desktop.log` files retain runtime logs. [SHA256SUMS](SHA256SUMS) hashes all raw artifacts.

Use the [runner](../../../../../../../scripts/verify-reading-experience-t7-star-performance.py) with a fresh output directory; system Python needs `websockets` and the native probe helper already in `scripts/`. Xvfb uses `-displayfd` to claim a free private display. The runner starts and stops only its own app and Xvfb processes.

```bash
PYTHONDONTWRITEBYTECODE=1 /usr/bin/python3 scripts/verify-reading-experience-t7-star-performance.py \
  --baseline /home/tcy/Github/RustRss-t7-artifacts/baseline-rustrss-desktop \
  --candidate /home/tcy/Github/RustRss/target/debug/rustrss-desktop \
  --fixture-source .chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/performance-star/fixture-200.sqlite \
  --out /tmp/rustrss-t7-star-new-run
```

The Tauri binary embeds its UI at build time, so these results apply to the two exact binary hashes above. The runner uses programmatic clicks inside native WebKitGTK to exercise production event handlers, IPC, reader rendering, star patching, and sidebar counts; it does not measure trusted pointer input, final GPU paint, Android performance, or cold article-open latency.
