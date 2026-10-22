// The appcast is rewritten by scripts/release-app.sh before every site build,
// so the version and requirements shown next to the button come from it.
import appcast from "../public/appcast.xml?raw"
import { latestRelease } from "./appcast"

export const MAC_DMG = "/releases/Scrawl-latest-arm64.dmg"
export const MAC_ZIP = "/releases/Scrawl-latest-arm64.zip"
export const BREW = "brew install --cask kitlangton/tap/scrawl"
export const SKILL = "bunx skills add kitlangton/scrawl"

// The zip .github/workflows/release-windows.yml builds for the same version,
// copied next to the Mac files. A preview: unsigned, and it doesn't update
// itself yet. Set it to null and the site says "coming soon" again.
export const WINDOWS_DOWNLOAD: string | null = "/releases/Scrawl-latest-windows-x64.zip"

export const RELEASE = latestRelease(appcast)

// Published by release-linux.yml. Build/deploy the site only after the tag's assets exist.
export const LINUX_DOWNLOAD: string | null = `https://github.com/kitlangton/scrawl/releases/download/v${RELEASE.version}/Scrawl-${RELEASE.version}-linux-x86_64.AppImage`
