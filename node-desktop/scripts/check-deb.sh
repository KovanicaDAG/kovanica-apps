#!/usr/bin/env bash
# Headless checks for the Debian 12 amd64 supervisor package.
# Prints SHA-256 checksums. Does not install the package and does not upload it.
# A missing copyright file is reported and is not treated as distribution readiness.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEB_DIR="$ROOT/src-tauri/target/release/bundle/deb"
shopt -s nullglob
DEBS=("$DEB_DIR"/*.deb)
if [[ "${#DEBS[@]}" -ne 1 ]]; then
  echo "expected exactly one .deb in $DEB_DIR, found ${#DEBS[@]}" >&2
  printf ' %s\n' "${DEBS[@]}" >&2
  exit 1
fi
DEB="${DEBS[0]}"
echo "deb=$DEB"
sha256sum "$DEB"
echo "=== control ==="
dpkg-deb -I "$DEB"

pkg="$(dpkg-deb -f "$DEB" Package)"
ver="$(dpkg-deb -f "$DEB" Version)"
arch="$(dpkg-deb -f "$DEB" Architecture)"
depends="$(dpkg-deb -f "$DEB" Depends)"
maint="$(dpkg-deb -f "$DEB" Maintainer)"
desc="$(dpkg-deb -f "$DEB" Description)"

[[ "$pkg" == "kovanica-node" ]] || { echo "Package is '$pkg', expected kovanica-node" >&2; exit 1; }
[[ "$ver" == "0.2.0" ]] || { echo "Version is '$ver', expected 0.2.0" >&2; exit 1; }
[[ "$arch" == "amd64" ]] || { echo "Architecture is '$arch', expected amd64" >&2; exit 1; }
[[ "$maint" == "Kovanica Team <KovanicaDAG@protonmail.com>" ]] || {
  echo "Maintainer is '$maint'" >&2
  exit 1
}
[[ "$desc" == *"read-only loopback monitor"* ]] || { echo "Description does not name the monitor: $desc" >&2; exit 1; }

for dep in libc6 libgcc-s1 libglib2.0-0 libgtk-3-0 libgdk-pixbuf-2.0-0 libcairo2 libdbus-1-3 libx11-6 libwebkit2gtk-4.1-0 libjavascriptcoregtk-4.1-0 libsoup-3.0-0; do
  if ! grep -Eq "(^|, )${dep}($|,| )" <<<"$depends"; then
    echo "Depends is missing $dep" >&2
    echo "$depends" >&2
    exit 1
  fi
done
grep -Fq "libc6 (>= 2.36)" <<<"$depends" || { echo "Depends missing libc6 (>= 2.36)" >&2; exit 1; }
if grep -Eq '(^|, )libssl3($|,| )' <<<"$depends"; then
  echo "Depends lists libssl3; the plan forbids that unless the release binary NEEDs it" >&2
  exit 1
fi

work="$(mktemp -d)"
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
dpkg-deb -e "$DEB" "$work/DEBIAN"
for script in preinst postinst prerm postrm; do
  if [[ -e "$work/DEBIAN/$script" ]]; then
    echo "maintainer script $script must not be present" >&2
    exit 1
  fi
done
dpkg-deb -x "$DEB" "$work/root"

echo "=== payload ==="
(cd "$work/root" && find . -print | sort)

if [[ -e "$work/root/etc" || -e "$work/root/var" || -e "$work/root/home" ]]; then
  echo "package installs outside /usr" >&2
  exit 1
fi
find "$work/root" -mindepth 1 -maxdepth 1 ! -name usr -print | grep -q . && {
  echo "top-level path other than usr" >&2
  exit 1
}
[[ -x "$work/root/usr/bin/kovanica-node-desktop" ]] || { echo "supervisor binary missing" >&2; exit 1; }
[[ ! -e "$work/root/usr/bin/kovanica-node" ]] || { echo "L1 binary must not be packaged" >&2; exit 1; }
if find "$work/root" \( -name 'genesis*' -o -name 'funded_accounts.toml' -o -name '*.toml' -o -name '*.db' -o -name 'wallet*' -o -name '*.key' \) | grep -q .; then
  echo "payload contains chain data, keys, or toml" >&2
  find "$work/root" \( -name 'genesis*' -o -name 'funded_accounts.toml' -o -name '*.toml' -o -name '*.db' -o -name 'wallet*' -o -name '*.key' \) >&2
  exit 1
fi
if find "$work/root" -path '*autostart*' -o -name '*.service' | grep -q .; then
  echo "autostart or systemd unit present" >&2
  exit 1
fi

shopt -s nullglob
desks=("$work/root/usr/share/applications"/*.desktop)
[[ "${#desks[@]}" -eq 1 ]] || { echo "expected one desktop file, found ${#desks[@]}" >&2; exit 1; }
desk="${desks[0]}"
echo "desktop=$desk"
cat "$desk"
grep -qx 'Exec=kovanica-node-desktop' "$desk"
grep -qx 'Terminal=false' "$desk"
grep -qx 'Type=Application' "$desk"
grep -qx 'Name=Kovanica Node' "$desk"
grep -q 'Utility' "$desk"
grep -Eq '^Categories=.*Utility' "$desk"
if grep -q '^Environment=' "$desk" || grep -q 'KOVANICA_' "$desk" || grep -q -- '--config' "$desk"; then
  echo "desktop file must not set KOVANICA_*, Environment=, or --config" >&2
  exit 1
fi
if grep -q 'X-GNOME-Autostart' "$desk"; then
  echo "desktop file must not autostart" >&2
  exit 1
fi
find "$work/root/usr/share/icons" -type f -name 'kovanica-node-desktop.png' | grep -q . || {
  echo "hicolor icon missing" >&2
  exit 1
}

bin="$work/root/usr/bin/kovanica-node-desktop"
echo "=== extracted binary ==="
sha256sum "$bin"
echo "=== NEEDED ==="
objdump -p "$bin" | awk '/NEEDED/ {print $2}'
echo "=== glibc symbol versions ==="
objdump -T "$bin" | awk '/GLIBC_/ {print $5}' | sed 's/.*GLIBC_//; s/).*//' | sort -Vu | tail -5

objdump -p "$bin" | awk '/NEEDED/ {print $2}' > "$work/needed"
while read -r so; do
  [[ -n "$so" ]] || continue
  path="$(ldconfig -p | awk -v so="$so" '$1==so {print $NF; exit}')"
  if [[ -z "$path" ]]; then
    echo "cannot map $so on this system" >&2
    exit 1
  fi
  owner="$(dpkg -S "$path" 2>/dev/null | head -1 | cut -d: -f1 || true)"
  if [[ -z "$owner" ]]; then
    alt="$path"
    if [[ "$path" == /lib/* ]]; then
      alt="/usr${path}"
    elif [[ "$path" == /usr/lib/* ]]; then
      alt="${path#/usr}"
    fi
    owner="$(dpkg -S "$alt" 2>/dev/null | head -1 | cut -d: -f1 || true)"
  fi
  if [[ -z "$owner" ]]; then
    echo "dpkg does not own $so at $path" >&2
    exit 1
  fi
  if ! grep -Eq "(^|, )${owner}($|,| )" <<<"$depends"; then
    echo "NEEDED $so is provided by $owner, which is not in Depends" >&2
    echo "$depends" >&2
    exit 1
  fi
  echo "needed $so -> $owner"
done < "$work/needed"

if [[ -d "$work/root/usr/share/doc" ]] && find "$work/root/usr/share/doc" -iname 'copyright' -type f | grep -q .; then
  echo "copyright file present"
else
  echo "LIMITATION: no copyright file. Cargo.toml license is GPL-3.0-or-later. This package is not distribution-ready."
fi
echo "check-deb: ok"
