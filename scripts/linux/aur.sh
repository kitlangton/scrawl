#!/usr/bin/env bash
# Prepares (and with --publish, pushes) the scrawl-linux-bin AUR package for a released version.
# Runs on Arch Linux as a non-root user, after both AppImages are attached to the v<version> release:
#
#   ./scripts/linux/aur.sh <version> [--publish]
#
# Writes PKGBUILD and .SRCINFO into target/aur/scrawl-linux-bin with the release's checksums, then
# builds the package and runs the installed AppImage to check it. --publish clones the AUR
# repository over SSH (the key comes from ssh-agent or ~/.ssh) and pushes the update. Publishing a
# version that's already there bumps pkgrel when the files changed and pushes nothing when they didn't.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
version="${1:?usage: aur.sh <version> [--publish]}"
version="${version#v}"
publish="${2:-}"
name=scrawl-linux-bin
out="$root/target/aur"
pkg="$out/$name"
build="$out/build"
export SRCDEST="$out/sources"

rm -rf "$pkg" "$build"
mkdir -p "$out" "$SRCDEST" "$build"

if [[ "$publish" == "--publish" ]]; then
  git clone "ssh://aur@aur.archlinux.org/$name.git" "$pkg"
else
  mkdir -p "$pkg"
fi

field() { [[ -f "$pkg/.SRCINFO" ]] && sed -n "s/^\t$1 = //p" "$pkg/.SRCINFO" | head -1 || true; }
published_version="$(field pkgver)"
published_rel="$(field pkgrel)"

render() {
  sed -e "s/^pkgver=.*/pkgver=$version/" -e "s/^pkgrel=.*/pkgrel=$1/" \
    "$root/packaging/aur/PKGBUILD" > "$pkg/PKGBUILD"
  cp "$root/packaging/aur/LICENSE" "$pkg/LICENSE"
  (cd "$pkg" && updpkgsums && makepkg --printsrcinfo > .SRCINFO)
}

if [[ "$published_version" == "$version" ]]; then
  render "$published_rel"
  if ! git -C "$pkg" diff --quiet; then
    # Same version, different files (a rebuilt release): users need a new pkgrel to upgrade.
    render "$((published_rel + 1))"
  fi
else
  render 1
fi

cp "$pkg/PKGBUILD" "$build/"
(cd "$build" && makepkg --force --syncdeps --noconfirm)
file="$(cd "$build" && makepkg --packagelist | head -1)"
tar -tf "$file" | grep -qx 'usr/bin/scrawl'
tmp="$(mktemp -d)"
tar -xf "$file" -C "$tmp"
APPIMAGE_EXTRACT_AND_RUN=1 "$tmp/opt/scrawl/Scrawl.AppImage" --help >/dev/null
rm -rf "$tmp"
echo "Built $file"

[[ "$publish" == "--publish" ]] || exit 0

cd "$pkg"
git add PKGBUILD .SRCINFO LICENSE
if git diff --cached --quiet; then
  echo "$name is already $version-$published_rel on the AUR"
  exit 0
fi
git -c user.name="${AUR_COMMIT_NAME:-Scrawl release}" -c user.email="${AUR_COMMIT_EMAIL:-dax@anoma.ly}" \
  commit -m "Update to $(field pkgver)-$(field pkgrel)"
git push origin HEAD:master
