// The release scripts (scripts/release-beta.sh, promote.sh) copy the appcast in before every
// site build, so the version and requirements shown next to the button come from its newest
// stable item.
import appcast from "../public/appcast.xml?raw"
import { latestRelease } from "./appcast"

export const MAC_DMG = "/releases/Scrawl-latest-arm64.dmg"
export const MAC_ZIP = "/releases/Scrawl-latest-arm64.zip"
export const BREW = "brew install --cask kitlangton/tap/scrawl"
export const SKILL = "bunx skills add kitlangton/scrawl"

// The zip .github/workflows/release-windows.yml builds for the same build,
// copied next to the Mac files on promotion. A preview: unsigned, and it doesn't update
// itself yet. Set it to null and the site says "coming soon" again.
export const WINDOWS_DOWNLOAD: string | null = "/releases/Scrawl-latest-windows-x64.zip"

export const RELEASE = latestRelease(appcast)
