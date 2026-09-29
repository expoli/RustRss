# v0.2.1 independent release review

Chorus task: `9ae99c22-6eda-42b8-ace6-aa5904aea773`  
Round 1 comment: `1b9af4fe-b0c1-4aa5-acb9-a8592c59b815`  
Created: `2026-09-29T07:16:58.072Z`

+### Review Summary

**PASS (4):** AC-1 version/scope/notes; AC-2 tests/lint/icon/CI; AC-3 signed universal APK and upgrade evidence; AC-4 public tag/assets and v0.2.0 preservation.

**Independent verification (Round 1):**

- Read task, all four ACs and all four task comments. This is a standalone quick release task (`proposalUuid=null`); parent-provided user intent authorizes a new 0.2.1 patch and continuation of commit/push/release.
- `git diff 36af3a4 6101f26` contains only CHANGELOG.md, Cargo.lock, Cargo.toml, README.md and src-tauri/tauri.conf.json. Workspace, all three inherited package versions, lock entries and Tauri are 0.2.1. Later 348d4ee/a669a0d contain retained evidence/docs only. Worktree clean; no extra product logic.
- Independently ran `cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo build --workspace --locked && python3 scripts/sync-icons.py --check`, exit 0. All workspace test suites report 0 failed; strict Clippy and build finish successfully; icon output: `All platform icon hashes, Android copies and launcher references match`.
- Live GitHub API confirms Source CI 36533525831, Android CI 36533524897 and release 36533942713 all success at exact `6101f269d0f428c3dab53595c592aad7d88acec1`. Release validate, macOS dmg, Ubuntu deb, Windows NSIS and release jobs each success. Annotated tag v0.2.1 peels to that commit. Public Release is draft=false/prerelease=false, has four uploaded platform assets, and notes accurately state the phone/OPML/icon fixes and verification boundaries: https://github.com/expoli/RustRss/releases/tag/v0.2.1.
- Independently hashed every actual retained downloaded file in `/tmp/rustrss-v021-downloaded-assets` and compared exact bytes + SHA256 against live GitHub asset digests: dmg 11340038 / 9220e66a353ea793a2b00ac47041f64cc67ba60f7c79df8c2a9a86acfd661321; deb 14231840 / 4669195f41052e429b8f1552fff9d3a4a3b6433dfdc4a3650c73dc9f8a89a989; APK 73050838 / b0e9474f7364b3713195e080ed87b99fb0cf29e6e09345e3aef6394076a8b6c1; exe 7530710 / 2cf66e65830792a44ea955721098d1f9dc0632fe1dd037669d8a369533f203f9. All match. `dpkg-deb -f` reports 0.2.1/amd64.
- Build-tools 36.0.0 `apksigner verify --verbose --print-certs` verifies downloaded APK signature; cert SHA256 is 89269c116afaa9ca546f7287b268f00c14d5598b6c633715a74fbbcd28de8cc5. Independently hashed the retained published 0.2.0 APK to b10b05f27f2d8d0c539cc99fb81120ba9859e735bb640617f08a5c1cdb0e0868 and verified the same certificate. `aapt dump badging` reports new 0.2.1/code2001 > old 0.2.0/code2000, arm64-v8a + x86_64, minSdk24 and no application-debuggable.
- Reviewed chronological developer upgrade script, real install-r Success log, old/new UID10231 and paired screenshots in `docs/releases/v0.2.1-android/`: published-020-data.png, v021-retained-data.png, v021-launcher.png, signed-fixed-opml-selectable.png. These establish retained upgrade/launcher/OPML observations; reviewer did not reinstall or replay the historical upgrade.
- Reviewer independently read the installed base.apk from owned `emulator-5580`: its SHA256 equals the downloaded public v0.2.1 APK, version 0.2.1/code2001 and UID10231. Native navigation and new screenshots confirm both existing Picker generic/Picker xml fixtures, visible warm Gboard URL input bounds [42,341,887,459] (bottom <900), and Settings category homepage. Evidence: `/tmp/rustrss-v021-review-r1-7w8a9if9/results.json`, subscriptions-visible.png/xml, warm-ime-reviewer.png/xml, settings-category-reviewer.png/xml. No uninstall, clear, import or fixture modification; device returned to Settings homepage.
- Live v0.2.0 annotated tag still peels to e1275a8505ba6cfdba499f1f9069fc23708b1506. Compared live four assets to both `/tmp/rustrss-v020-before-v021.json` and after snapshot: node IDs, names, byte sizes, SHA256 digests, API URLs and download URLs all unchanged.
- Known bounds are accurately retained: physical ARM64, third-party IME/document providers/launchers and Windows/macOS native runtime unverified; macOS dmg unsigned; iOS icons only. No new defect found within this version-only release task.

**NOTE (0):** None.

**BLOCKER (0):** None.

VERDICT: PASS

Retained reviewer device evidence: [independent/results.json](independent/results.json).
The reviewer inspected the actual historical upgrade evidence and independently
checked the installed public APK and current UI; the historical upgrade was not rerun.
