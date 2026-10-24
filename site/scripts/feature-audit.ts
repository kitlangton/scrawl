/// <reference types="node" />
// Build-time checks for the feature map. Shape errors fail the build; the rest
// are shown on the page under "Needs a look". The config-key and experiment
// check runs in `cargo test` (src/feature_map.rs), where CI sees it.
import { existsSync, readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { FEATURES, parseStatus, shapeErrors, type Feature } from "../src/features.ts"

/** The map's unlinked path on the site. Not in the sitemap, llms.txt or any nav. */
export const MAP_DIR = "map-tyyxsf5b"

/** An experiment older than this should be shipped or dropped. */
export const EXPERIMENT_WEEKS = 6
/** A PR open longer than this needs a decision. */
export const REVIEW_DAYS = 14

export type Issue = { feature?: string; text: string }
export type Audit = {
  release: { version: string; date: string }
  today: string
  errors: string[]
  issues: Issue[]
}

const repoPath = (path: string) => fileURLToPath(new URL(`../../${path}`, import.meta.url))
const DAY = 86_400_000

// The newest stable appcast item: its version and the day it was published.
export function latestRelease(xml: string): { version: string; date: string } {
  let best = { build: -1, version: "", date: "" }
  for (const item of xml.split("<item>").slice(1)) {
    if (/<sparkle:channel>[^<]+<\/sparkle:channel>/.test(item)) continue
    const build = Number(item.match(/<sparkle:version>(\d+)</)?.[1] ?? -1)
    const version = item.match(/<sparkle:shortVersionString>([^<]+)</)?.[1] ?? ""
    const pub = item.match(/<pubDate>([^<]+)</)?.[1]
    if (build > best.build && version && pub) best = { build, version, date: pubDay(pub) }
  }
  return { version: best.version, date: best.date }
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

// The day in the pubDate's own time zone ("Wed, 21 Oct 2026 17:33:07 -0700" is the 21st).
function pubDay(pub: string): string {
  const m = pub.match(/(\d{1,2}) (\w{3}) (\d{4})/)
  if (!m) return ""
  return `${m[3]}-${String(MONTHS.indexOf(m[2]) + 1).padStart(2, "0")}-${m[1].padStart(2, "0")}`
}

const isoDay = (d: Date) => d.toLocaleDateString("en-CA")
const days = (from: string, to: string) => Math.round((Date.parse(to) - Date.parse(from)) / DAY)

function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number)
  const pb = b.split(".").map(Number)
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return (pa[i] ?? 0) - (pb[i] ?? 0)
  return 0
}

// GitHub's heading anchors: lower case, punctuation dropped, spaces to hyphens.
export function anchors(markdown: string): Set<string> {
  const out = new Set<string>()
  let fenced = false
  for (const line of markdown.split("\n")) {
    if (line.startsWith("```")) fenced = !fenced
    const h = !fenced && line.match(/^#{1,6} (.+)$/)
    if (h) out.add(h[1].trim().toLowerCase().replace(/[^\p{L}\p{N} _-]/gu, "").replace(/ /g, "-"))
  }
  return out
}

function docIssues(f: Feature): Issue[] {
  const issues: Issue[] = []
  for (const doc of f.docs ?? []) {
    const [path, anchor] = doc.split("#")
    if (!existsSync(repoPath(path))) {
      issues.push({ feature: f.id, text: `links ${path}, which isn't in the repo` })
    } else if (anchor && !anchors(readFileSync(repoPath(path), "utf8")).has(anchor)) {
      issues.push({ feature: f.id, text: `links ${doc}, but ${path} has no such heading` })
    }
  }
  return issues
}

export function audit(now = new Date()): Audit {
  const release = latestRelease(readFileSync(repoPath("site/public/appcast.xml"), "utf8"))
  const today = isoDay(now)
  const errors = shapeErrors()
  const issues: Issue[] = []
  if (errors.length) return { release, today, errors, issues }

  for (const f of FEATURES) {
    const status = parseStatus(f.status)
    issues.push(...docIssues(f))
    if (status.kind === "stable") {
      if (release.version && compareVersions(status.version, release.version) > 0) {
        issues.push({ feature: f.id, text: `says stable@${status.version}, but the newest release is ${release.version}` })
      }
      if (f.group !== "platforms" && !f.platforms.includes("windows") && !f.note) {
        issues.push({ feature: f.id, text: "is stable but not on Windows, with no note saying why" })
      }
    }
    if (status.kind === "main" && f.added && release.date && f.added < release.date) {
      issues.push({ feature: f.id, text: `landed ${f.added}, before ${release.version} (${release.date}); stable@${release.version}?` })
    }
    if (status.kind === "experiment" && f.added && days(f.added, today) > EXPERIMENT_WEEKS * 7) {
      issues.push({ feature: f.id, text: `has been an experiment for ${Math.floor(days(f.added, today) / 7)} weeks; ship it or drop it` })
    }
    if (status.kind === "review" && f.added && days(f.added, today) > REVIEW_DAYS) {
      issues.push({ feature: f.id, text: `PR #${status.pr} has been open ${days(f.added, today)} days` })
    }
  }
  return { release, today, errors, issues }
}

export { days }
