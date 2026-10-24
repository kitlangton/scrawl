export type Release = { version: string; build: number; macos: number }

// Public downloads follow stable, even when a newer opt-in beta exists.
export function latestRelease(xml: string): Release {
  let best: Release | null = null
  for (const item of xml.split("<item>").slice(1)) {
    const tag = (name: string) => item.match(new RegExp(`<sparkle:${name}>([^<]+)</sparkle:${name}>`))?.[1]
    if (tag("channel")) continue
    const build = Number(tag("version") ?? 0)
    const version = tag("shortVersionString")
    if (!version || (best && build <= best.build)) continue
    best = { build, version, macos: parseInt(tag("minimumSystemVersion") ?? "15", 10) }
  }
  return best ?? { version: "", build: 0, macos: 15 }
}
