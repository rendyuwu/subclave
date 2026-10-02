#!/usr/bin/env bash
#
# Post-build smoke check: assert that `tauri build` actually emitted every
# installer this platform is configured to produce. A Tauri build can succeed
# while silently skipping a bundle target (missing tool, unsupported target
# triple), and an artifact set that is missing the one installer the reviewer
# wanted to smoke-test is worse than a red build.
#
# The expected sets below mirror the `bundle.targets` in the tauri configs:
#   tauri.linux.conf.json    -> deb, rpm, appimage
#   tauri.windows.conf.json  -> nsis only (deliberately no MSI)
#   tauri.conf.json          -> "all", which on macOS means app + dmg
# Keep them in sync when a target is added or dropped.
#
# Usage: verify-bundles.sh <bundle-root> <matrix-id>
#   bundle-root  e.g. src-tauri/target/release/bundle
#   matrix-id    linux-x64 | windows-x64 | macos-x64 | macos-arm64

set -euo pipefail

root="${1:?verify-bundles: bundle root argument is required}"
id="${2:?verify-bundles: matrix id argument is required}"

die() {
  printf '::error::%s\n' "$*" >&2
  exit 1
}

[ -d "$root" ] || die "verify-bundles: bundle root '$root' does not exist — tauri build produced nothing."

# "<human label>|<find -name pattern>"
case "$id" in
  linux-*)
    # The AppImage is also the updater target on Linux, so its .sig is checked
    # separately below: a missing signature means the updater artifacts were not
    # produced even though the installer was.
    expected=("AppImage|*.AppImage" "AppImage signature|*.AppImage.sig" \
      "Debian package|*.deb" "RPM package|*.rpm")
    ;;
  windows-*)
    expected=("NSIS installer|*-setup.exe" "NSIS signature|*-setup.exe.sig")
    ;;
  macos-*)
    # .app is a directory, not a file, so these checks deliberately do not
    # constrain -type. The .app.tar.gz is the updater artifact.
    expected=("disk image|*.dmg" "app bundle|*.app" \
      "updater tarball|*.app.tar.gz" "updater signature|*.app.tar.gz.sig")
    ;;
  *)
    die "verify-bundles: unknown matrix id '$id'."
    ;;
esac

missing=0
for entry in "${expected[@]}"; do
  label="${entry%%|*}"
  pattern="${entry#*|}"
  found="$(find "$root" -name "$pattern" 2>/dev/null || true)"
  if [ -z "$found" ]; then
    printf '::error::Missing %s for %s — no %s under %s\n' "$label" "$id" "$pattern" "$root" >&2
    missing=1
  else
    while IFS= read -r path; do
      printf 'ok  %-18s %s\n' "$label" "$path"
    done <<<"$found"
  fi
done

if [ "$missing" -ne 0 ]; then
  printf '\nContents of %s:\n' "$root" >&2
  find "$root" -maxdepth 2 >&2 || true
  die "verify-bundles: one or more expected bundles were not produced for $id."
fi

# The native messaging sidecar must be inside the installers, not merely staged
# next to the build: `proxy_path()` in the Rust manifest writer looks for it at
# exactly these locations, so a silent drop here leaves a dead browser channel.
# Each Linux check runs only when its extraction tool is present, so a runner
# without `rpm` warns instead of failing a bundle that is actually fine.
#
# The listings are captured and matched with `case`, never piped into `grep -q`:
# this script runs with `pipefail`, and `grep -q` closes the pipe the moment it
# matches, so the tool on the left dies of EPIPE and the pipeline reports
# failure even though the sidecar was found.
case "$id" in
  linux-*)
    deb="$(find "$root" -name '*.deb' 2>/dev/null | head -n1 || true)"
    if ! command -v dpkg >/dev/null 2>&1; then
      printf '::warning::verify-bundles: dpkg unavailable; skipping the deb sidecar check.\n' >&2
    else
      listing="$(dpkg -c "$deb" 2>/dev/null || true)"
      case "$listing" in
        *usr/bin/subclave-proxy*) printf 'ok  %-18s %s\n' "sidecar in deb" "$deb" ;;
        *) die "verify-bundles: the deb is missing usr/bin/subclave-proxy ($deb)." ;;
      esac
    fi

    rpm_pkg="$(find "$root" -name '*.rpm' 2>/dev/null | head -n1 || true)"
    if ! command -v rpm >/dev/null 2>&1; then
      printf '::warning::verify-bundles: rpm unavailable; skipping the rpm sidecar check.\n' >&2
    else
      listing="$(rpm -qlp "$rpm_pkg" 2>/dev/null || true)"
      case "$listing" in
        *usr/bin/subclave-proxy*) printf 'ok  %-18s %s\n' "sidecar in rpm" "$rpm_pkg" ;;
        *) die "verify-bundles: the rpm is missing usr/bin/subclave-proxy ($rpm_pkg)." ;;
      esac
    fi

    # The AppImage's AppDir is assembled from the same `usr/` tree the deb is
    # (Tauri's linuxdeploy step copies the deb data dir into the AppDir), so the
    # deb check above already covers the sidecar's presence. This runs the
    # AppImage's own extractor as a direct check, and warns rather than fails
    # when the runtime cannot extract at all on a runner (no FUSE, a noexec
    # tmp): that is a tooling problem, not a packaging one. A successful
    # extraction that lacks the file is still a failure.
    appimage="$(find "$root" -name '*.AppImage' 2>/dev/null | head -n1 || true)"
    if [ -z "$appimage" ]; then
      printf '::warning::verify-bundles: no AppImage to inspect for the sidecar.\n' >&2
    else
      # CI passes a relative root, and the extractor runs from a temp dir, so
      # resolve the path before the `cd` or it stops resolving.
      appimage="$(cd "$(dirname "$appimage")" && pwd)/$(basename "$appimage")"
      extract_dir="$(mktemp -d)"
      extract_log="$( cd "$extract_dir" && "$appimage" --appimage-extract 2>&1 || true )"
      if [ -f "$extract_dir/squashfs-root/usr/bin/subclave-proxy" ]; then
        printf 'ok  %-18s %s\n' "sidecar in appimage" "$appimage"
      elif [ -d "$extract_dir/squashfs-root" ]; then
        rm -rf "$extract_dir"
        die "verify-bundles: the AppImage is missing usr/bin/subclave-proxy ($appimage); extractor said: $(printf '%s' "$extract_log" | tail -n 3 | tr '\n' ' ')"
      else
        printf '::warning::verify-bundles: the AppImage could not be extracted on this runner: %s\n' "$(printf '%s' "$extract_log" | tail -n 3 | tr '\n' ' ')" >&2
      fi
      rm -rf "$extract_dir"
    fi
    ;;
  macos-*)
    app="$(find "$root" -name '*.app' 2>/dev/null | head -n1 || true)"
    if [ -f "$app/Contents/MacOS/subclave-proxy" ]; then
      printf 'ok  %-18s %s\n' "sidecar in .app" "$app"
    else
      die "verify-bundles: the .app is missing Contents/MacOS/subclave-proxy ($app)."
    fi
    ;;
  windows-*)
    # The NSIS payload needs 7z to list; skip with a note when the runner has no
    # extractor rather than failing on tooling.
    exe="$(find "$root" -name '*-setup.exe' 2>/dev/null | head -n1 || true)"
    if [ -n "$exe" ] && command -v 7z >/dev/null 2>&1; then
      listing="$(7z l "$exe" 2>/dev/null || true)"
      case "$listing" in
        *subclave-proxy.exe*) printf 'ok  %-18s %s\n' "sidecar in nsis" "$exe" ;;
        *) die "verify-bundles: the NSIS payload is missing subclave-proxy.exe ($exe)." ;;
      esac
    else
      printf 'skip %-18s no 7z on this runner\n' "sidecar in nsis"
    fi
    ;;
esac

printf 'All expected bundles present for %s.\n' "$id"
