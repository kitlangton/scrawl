import { useId, useState } from "react"
import { Copy } from "./Copy"
import { usePlatform, type Platform } from "./platform"
import { BREW, LINUX_DOWNLOAD, MAC_DMG, RELEASE, WINDOWS_DOWNLOAD } from "./release"

const SITE = "https://scrawl.kitlangton.com/"
const REQUIRES = `Apple silicon · macOS ${RELEASE.macos} or later`

function Apple() {
  return (
    <svg viewBox="0 0 17 21" aria-hidden>
      <path d="M11.2 4.3c.6-.8 1-1.8.9-2.8-.9 0-2 .6-2.6 1.4-.6.7-1.1 1.7-.9 2.7 1 .1 2-.5 2.6-1.3zM12.1 5.8c-1.4-.1-2.7.8-3.3.8-.7 0-1.7-.8-2.9-.8C4.4 5.9 3 6.8 2.2 8.1c-1.6 2.7-.4 6.8 1.1 9 .8 1.1 1.7 2.3 2.8 2.3 1.1 0 1.6-.7 2.9-.7 1.4 0 1.8.7 2.9.7 1.2 0 2-1.1 2.7-2.2.9-1.3 1.2-2.5 1.2-2.6 0 0-2.4-.9-2.4-3.6 0-2.2 1.8-3.3 1.9-3.4-1-1.5-2.6-1.7-3.2-1.7z" />
    </svg>
  )
}

function Windows() {
  return (
    <svg viewBox="0 0 16 16" aria-hidden className="icon-windows">
      <path d="M0 0h7.6v7.6H0zM8.4 0H16v7.6H8.4zM0 8.4h7.6V16H0zM8.4 8.4H16V16H8.4z" />
    </svg>
  )
}

function Share() {
  return (
    <svg viewBox="0 0 16 20" aria-hidden className="icon-share">
      <path d="M8 1v11M4.5 4.5 8 1l3.5 3.5M5.5 8H3.5A1.5 1.5 0 0 0 2 9.5v7A1.5 1.5 0 0 0 3.5 18h9a1.5 1.5 0 0 0 1.5-1.5v-7A1.5 1.5 0 0 0 12.5 8h-2" />
    </svg>
  )
}

function Chevron() {
  return (
    <svg viewBox="0 0 10 6" aria-hidden className="chevron">
      <path d="M1 1l4 4 4-4" />
    </svg>
  )
}

function Unavailable({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <span className="btn-primary btn-unavailable" aria-disabled="true">
      {icon}
      {label}
    </span>
  )
}

function SendLink() {
  const [copied, setCopied] = useState(false)
  const canShare = typeof window !== "undefined" && typeof navigator.share === "function"
  return (
    <button
      className="btn-primary"
      onClick={async () => {
        if (canShare) {
          await navigator.share({ title: "Scrawl", text: "Scrawl for Mac, Windows and Linux", url: SITE }).catch(() => {})
          return
        }
        await navigator.clipboard?.writeText(SITE)
        setCopied(true)
        setTimeout(() => setCopied(false), 1400)
      }}
    >
      <Share />
      {canShare ? "Send yourself the link" : copied ? "Link copied" : "Copy link for your computer"}
    </button>
  )
}

function Primary({ platform }: { platform: Platform }) {
  switch (platform.kind) {
    case "mac":
      return (
        <a className="btn-primary" href={MAC_DMG}>
          <Apple />
          Download for Mac
        </a>
      )
    case "mac-intel":
      return <Unavailable icon={<Apple />} label="Download for Mac" />
    case "windows":
      return WINDOWS_DOWNLOAD ? (
        <a className="btn-primary" href={WINDOWS_DOWNLOAD}>
          <Windows />
          Download for Windows
        </a>
      ) : (
        <Unavailable icon={<Windows />} label="Windows: coming soon" />
      )
    case "linux":
      return LINUX_DOWNLOAD ? (
        <a className="btn-primary" href={LINUX_DOWNLOAD}>Download for Linux</a>
      ) : <Unavailable icon={null} label="Linux: coming soon" />
    case "mobile":
      return <SendLink />
  }
}

function meta(platform: Platform): string {
  switch (platform.kind) {
    case "mac":
      if (platform.macos && platform.macos < RELEASE.macos)
        return `Needs macOS ${RELEASE.macos} or later. This Mac has macOS ${platform.macos}.`
      return `Version ${RELEASE.version} · ${REQUIRES}`
    case "mac-intel":
      return `Needs a Mac with Apple silicon and macOS ${RELEASE.macos} or later.`
    case "windows":
      return WINDOWS_DOWNLOAD
        ? `Preview ${RELEASE.version} · Windows 10 and 11, x64 · Not signed yet: if Windows warns, click More info, then Run anyway.`
        : "Scrawl is Mac-only today. Windows is on the way."
    case "linux":
      return LINUX_DOWNLOAD ? `Preview ${RELEASE.version} · x86_64 AppImage · Ubuntu 24.04+ · X11 or GNOME/KDE Wayland` : "Linux is on the way."
    case "mobile":
      return WINDOWS_DOWNLOAD ? "Scrawl runs on your Mac or PC." : "Scrawl runs on your Mac."
  }
}

function Soon() {
  return <span className="soon">Soon</span>
}

function LinuxNotes() {
  const command = 'sudo usermod -aG input "$USER"'
  return (
    <div className="linux-notes">
      <p>On Wayland, enable clicks and Shift ink, then log out and back in:</p>
      <div className="platform-brew"><code>{command}</code><Copy text={command} /></div>
      <p>If the AppImage asks for libfuse.so.2, install <code>libfuse2t64</code> on Ubuntu 24.04, or run it with <code>--appimage-extract-and-run</code>.</p>
    </div>
  )
}

export function Platforms({ brew }: { brew: boolean }) {
  return (
    <ul className="platform-list">
      <li>
        <div className="platform-row">
          <span className="platform-name">Mac</span>
          <span className="platform-detail">Apple silicon, macOS {RELEASE.macos}+</span>
          <a className="platform-action" href={MAC_DMG}>
            Download
          </a>
        </div>
        {brew && (
          <div className="platform-brew">
            <code>
              <span className="dim">$ </span>
              {BREW}
            </code>
            <Copy text={BREW} />
          </div>
        )}
      </li>
      <li>
        <div className="platform-row">
          <span className="platform-name">Windows</span>
          <span className="platform-detail">{WINDOWS_DOWNLOAD ? "x64, preview, unsigned" : "x64"}</span>
          {WINDOWS_DOWNLOAD ? (
            <a className="platform-action" href={WINDOWS_DOWNLOAD}>
              Download
            </a>
          ) : (
            <Soon />
          )}
        </div>
      </li>
      <li>
        <div className="platform-row">
          <span className="platform-name">Linux</span>
          <span className="platform-detail">x86_64 AppImage, preview</span>
          {LINUX_DOWNLOAD ? <a className="platform-action" href={LINUX_DOWNLOAD}>Download</a> : <Soon />}
        </div>
      </li>
    </ul>
  )
}

export function Download({ brew = true }: { brew?: boolean }) {
  const platform = usePlatform()
  const [open, setOpen] = useState(false)
  const [touched, setTouched] = useState(false)
  const id = useId()
  return (
    <div className="download" data-platform={platform.kind}>
      <Primary platform={platform} />
      <p className="download-meta" key={platform.kind}>
        {meta(platform)}
      </p>
      {platform.kind === "linux" && LINUX_DOWNLOAD && <LinuxNotes />}
      <button
        className="download-more"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => {
          setTouched(true)
          setOpen(!open)
        }}
      >
        Other platforms
        <Chevron />
      </button>
      <div className="platforms" id={id} data-open={open} data-touched={touched} inert={!open}>
        <div className="platforms-clip">
          <Platforms brew={brew} />
        </div>
      </div>
    </div>
  )
}

export function useDownloadHref(): string {
  const platform = usePlatform()
  if (platform.kind === "mac") return MAC_DMG
  if (platform.kind === "windows" && WINDOWS_DOWNLOAD) return WINDOWS_DOWNLOAD
  if (platform.kind === "linux" && LINUX_DOWNLOAD) return LINUX_DOWNLOAD
  return "#install"
}
