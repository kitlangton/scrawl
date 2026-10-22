/// <reference types="node" />
// Builds llms.txt, llms-full.txt and skill.md from the README, the agent skill
// and the appcast, so they change when those do. vite.config.ts emits them
// into dist and serves them in dev.
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { latestRelease } from "../src/appcast.ts"

const SITE = "https://scrawl.kitlangton.com"
const REPO = "https://github.com/kitlangton/scrawl"
const RAW_SKILL = "https://raw.githubusercontent.com/kitlangton/scrawl/main/skills/scrawl/SKILL.md"
const DMG = `${SITE}/releases/Scrawl-latest-arm64.dmg`
const WINDOWS_ZIP = `${SITE}/releases/Scrawl-latest-windows-x64.zip`
const linuxDownload = (version: string) => `${REPO}/releases/download/v${version}/Scrawl-${version}-linux-x86_64.AppImage`

const repo = (path: string) => readFileSync(fileURLToPath(new URL(`../../${path}`, import.meta.url)), "utf8")

export function release() {
  return latestRelease(repo("site/public/appcast.xml"))
}

// Lines outside fenced code blocks, so a "# comment" in a shell block isn't a heading.
function mapProse(md: string, f: (line: string) => string): string {
  let fenced = false
  return md
    .split("\n")
    .map((line) => {
      if (line.startsWith("```")) fenced = !fenced
      return fenced || line.startsWith("```") ? line : f(line)
    })
    .join("\n")
}

function section(md: string, heading: string): string {
  const level = heading.match(/^#+/)![0].length
  const out: string[] = []
  let inside = false
  let fenced = false
  for (const line of md.split("\n")) {
    if (line.startsWith("```")) fenced = !fenced
    const h = !fenced && line.match(/^(#+) /)
    if (h && inside && h[1].length <= level) break
    if (inside) out.push(line)
    if (line === heading) inside = true
  }
  if (!inside) throw new Error(`agent-files: no "${heading}" section`)
  return out.join("\n").trim()
}

const shift = (md: string, by: number) => mapProse(md, (l) => (/^#+ /.test(l) ? "#".repeat(by) + l : l))
const unfrontmatter = (md: string) => md.replace(/^---\n[\s\S]*?\n---\n+/, "")

// One line per subcommand in the skill's shell examples. A command the map
// doesn't know yet falls back to its own comment in the skill.
const SAYS: Record<string, string> = {
  zoom: "a PNG sheet of frames around a moment, given as M:SS or as a phrase that was said",
  inspect: "print summary.md, or scrawl.json with --json",
  frames: "plain PNGs of a time range; prints their paths",
  send: "queue the capture into an OpenCode session, or --clipboard to copy the summary",
  export: "one .mp4 with the ink and clicks burned in and the audio in sync",
  skill: "print the agent skill without installing anything",
}

function commands(skill: string): string[] {
  const seen = new Map<string, string>()
  let fenced = false
  for (const line of skill.split("\n")) {
    if (line.startsWith("```")) fenced = !fenced
    const m = fenced && line.match(/^(scrawl (\w+)[^#]*?)\s*(?:#\s*(.+))?$/)
    if (m && !seen.has(m[2])) seen.set(m[2], `- \`${m[1]}\`: ${SAYS[m[2]] ?? m[3] ?? ""}`.replace(/: $/, ""))
  }
  if (!seen.has("skill")) seen.set("skill", `- \`scrawl skill\`: ${SAYS.skill}`)
  return [...seen.values()]
}

export function agentFiles(): Record<string, string> {
  const readme = repo("README.md")
  const skill = repo("skills/scrawl/SKILL.md")
  const { version, macos } = release()
  const requires = `Apple silicon Mac, macOS ${macos} or later`
  const tagline = readme.split("\n").find((l) => l && !l.startsWith("#"))!.trim()
  const intro = readme.split(/\n## /)[0].split("\n\n").slice(2).join("\n\n").trim()
  const summary = `> ${tagline} Scrawl records your screen, voice, clicks and drawings and packs them into a bundle an AI coding agent can read. Version ${version}, for an ${requires}, with previews for Windows 10 and 11 (x64) and Linux (Ubuntu 24.04+, X11 or GNOME/KDE Wayland).`

  const llms = `# Scrawl

${summary}

${intro}

## Install

- [Download for Mac](${DMG}): the DMG of version ${version}, for an ${requires}. The app updates itself with Sparkle.
- [Homebrew tap](https://github.com/kitlangton/homebrew-tap): \`brew install --cask kitlangton/tap/scrawl\`.
- [Download for Windows](${WINDOWS_ZIP}): a zip with \`scrawl.exe\` for Windows 10 (2004) and later on x64, version ${version}. It's a preview and isn't signed yet, so SmartScreen may warn ("More info", then "Run anyway"), and it doesn't update itself.
- [Download for Linux](${linuxDownload(version)}): x86_64 AppImage preview. Make it executable and run it with \`app\`. On Wayland, run \`sudo usermod -aG input "$USER"\`, then log out and back in for clicks and Shift ink. Ubuntu 24.04 uses \`libfuse2t64\`; \`--appimage-extract-and-run\` works without FUSE. ARM64 is also available on the GitHub release.

## Agents

- [Agent skill](${SITE}/skill.md): install it with \`bunx skills add kitlangton/scrawl\`. With it, an agent that gets a Scrawl paste reads the contact sheet and transcript, opens the keyframes the narration points at, and uses the \`scrawl\` command to look closer.
- [SKILL.md on GitHub](${RAW_SKILL}): the same skill, raw, from the main branch.
- [Full text](${SITE}/llms-full.txt): this file plus the install notes, capture controls, CLI and the whole skill in one document.

## Captures

- [Reading a capture](${SITE}/skill.md): what a paste looks like, the files in a \`.scrawl\` bundle (\`contact-sheet.png\`, \`frames/frame-NN.png\`, \`summary.md\`, \`scrawl.json\`, the recording and \`audio.wav\`), and what the red marks, click ripples and laser trails mean.

## CLI

The \`scrawl\` command is the binary inside Scrawl.app (on Windows, \`scrawl.exe\`) and runs on the computer where Scrawl is installed. Most commands take a bundle path or \`latest\`.

${commands(skill).join("\n")}

## Optional

- [README](${REPO}#readme): everything else, including the iPhone capture, ScrawlKit for iOS apps, auto-crop, sending dev-site captures to OpenCode sessions, and retention.
- [Source code](${REPO}): Rust and Swift, MIT licensed.
- [Appcast](${SITE}/appcast.xml): the Sparkle update feed.
`

  const full = `# Scrawl

${summary}

${intro}

Website: ${SITE}
Source: ${REPO}
Download: ${DMG} (Mac), ${WINDOWS_ZIP} (Windows preview), ${linuxDownload(version)} (Linux preview)

## Install

${section(readme, "## Install")}

## Agents

${section(readme, "## Agents")}

## Capture controls

${section(readme, "## Capture Controls")}

## Export as video

${section(readme, "## Export as video")}

## Keeping captures small

${section(readme, "## Keeping captures small")}

## The agent skill

This is the skill that \`bunx skills add kitlangton/scrawl\` installs, also served at <${SITE}/skill.md>.

${shift(unfrontmatter(skill), 2).trim()}
`

  return { "llms.txt": llms, "llms-full.txt": full, "skill.md": skill }
}

export function jsonLd(): string {
  const { version, macos } = release()
  return JSON.stringify({
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Scrawl",
    description: "Show an agent a problem instead of describing it. Scrawl records your screen, voice, clicks and drawings and packs them into a bundle an AI coding agent can read.",
    url: `${SITE}/`,
    operatingSystem: `macOS ${macos}, Windows 10, Windows 11, Linux`,
    applicationCategory: "DeveloperApplication",
    softwareVersion: version,
    downloadUrl: [DMG, WINDOWS_ZIP, linuxDownload(version)],
    image: `${SITE}/og.png`,
    offers: { "@type": "Offer", price: "0", priceCurrency: "USD" },
    author: { "@type": "Person", name: "Kit Langton" },
    sameAs: [REPO],
  })
}
