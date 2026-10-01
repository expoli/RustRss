#!/usr/bin/env python3
"""Verify source versions and, optionally, an Android APK's packaged version."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import tomllib


ROOT = Path(__file__).resolve().parent.parent
SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.-]+)?$")


def source_version() -> tuple[str, tuple[int, int, int]]:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    cargo_version = cargo["workspace"]["package"]["version"]
    tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    tauri_version = tauri["version"]
    if cargo_version != tauri_version:
        raise SystemExit(
            f"Version mismatch: Cargo workspace is {cargo_version}, "
            f"Tauri config is {tauri_version}"
        )
    match = SEMVER.fullmatch(cargo_version)
    if not match:
        raise SystemExit(f"Application version is not valid semver: {cargo_version}")
    return cargo_version, tuple(map(int, match.groups()))


def android_version_code(parts: tuple[int, int, int]) -> int:
    major, minor, patch = parts
    code = major * 1_000_000 + minor * 1_000 + patch
    if not 1 <= code <= 2_100_000_000:
        raise SystemExit(f"Derived Android versionCode is out of range: {code}")
    return code


def verify_apk(apk: Path, aapt: Path, version: str, parts: tuple[int, int, int]) -> None:
    if not apk.is_file():
        raise SystemExit(f"Android APK does not exist: {apk}")
    if not aapt.is_file():
        raise SystemExit(f"aapt does not exist: {aapt}")
    badging = subprocess.check_output([str(aapt), "dump", "badging", str(apk)], text=True)
    package_line = badging.splitlines()[0] if badging else ""
    name_match = re.search(r"versionName='([^']+)'", package_line)
    code_match = re.search(r"versionCode='(\d+)'", package_line)
    if not name_match or not code_match:
        raise SystemExit("Could not read versionName/versionCode from APK badging")
    packaged_name = name_match.group(1)
    packaged_code = int(code_match.group(1))
    expected_code = android_version_code(parts)
    if packaged_name != version or packaged_code != expected_code:
        raise SystemExit(
            "Android APK version mismatch: "
            f"expected {version}/{expected_code}, got {packaged_name}/{packaged_code}"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected", help="Expected app version, with an optional leading v")
    parser.add_argument("--apk", type=Path, help="Android APK to inspect")
    parser.add_argument("--aapt", type=Path, help="aapt binary used with --apk")
    args = parser.parse_args()

    version, parts = source_version()
    if args.expected and args.expected.removeprefix("v") != version:
        raise SystemExit(f"Expected version {args.expected}, source version is {version}")
    if bool(args.apk) != bool(args.aapt):
        raise SystemExit("--apk and --aapt must be provided together")
    if args.apk:
        verify_apk(args.apk, args.aapt, version, parts)
        print(f"Version consistency OK: app={version}, androidCode={android_version_code(parts)}")
    else:
        print(f"Version consistency OK: app={version}")


if __name__ == "__main__":
    main()
