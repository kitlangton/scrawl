import { useEffect, useState } from "react"

const REPO = "https://github.com/tryapparent/apparent"

export function GithubStars() {
  const [stars, setStars] = useState<number | null>(null)

  useEffect(() => {
    const controller = new AbortController()
    const timeout = setTimeout(() => controller.abort(), 5000)
    fetch("https://api.github.com/repos/tryapparent/apparent", {
      signal: controller.signal,
      credentials: "omit",
      headers: { Accept: "application/vnd.github+json" },
    })
      .then((response) => response.ok ? response.json() : null)
      .then((data) => {
        if (!controller.signal.aborted && Number.isSafeInteger(data?.stargazers_count) && data.stargazers_count >= 0) {
          setStars(data.stargazers_count)
        }
      })
      .catch(() => { /* Keep the GitHub link when offline or rate-limited. */ })
      .finally(() => clearTimeout(timeout))
    return () => {
      controller.abort()
      clearTimeout(timeout)
    }
  }, [])

  const count = stars === null ? "GitHub" : new Intl.NumberFormat("en", {
    notation: "compact", maximumFractionDigits: 1,
  }).format(stars)

  return (
    <a className="github-stars" href={REPO}
      aria-label={stars === null ? "Star Apparent on GitHub" : `Apparent on GitHub: ${stars.toLocaleString("en")} stars`}
      title={stars === null ? "Star Apparent on GitHub" : `${stars.toLocaleString("en")} stars on GitHub`}>
      <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="M12 .75a11.25 11.25 0 0 0-3.56 21.92c.56.1.77-.24.77-.54v-2.1c-3.13.68-3.79-1.33-3.79-1.33-.51-1.3-1.25-1.65-1.25-1.65-1.02-.7.08-.68.08-.68 1.13.08 1.72 1.16 1.72 1.16 1 1.72 2.64 1.22 3.28.93.1-.72.39-1.22.71-1.5-2.5-.28-5.13-1.25-5.13-5.56 0-1.23.44-2.23 1.16-3.01-.12-.29-.5-1.43.11-2.98 0 0 .95-.3 3.1 1.15a10.8 10.8 0 0 1 5.64 0c2.15-1.46 3.1-1.15 3.1-1.15.61 1.55.23 2.69.11 2.98.72.78 1.16 1.78 1.16 3.01 0 4.32-2.63 5.27-5.14 5.55.4.35.76 1.03.76 2.08v3.1c0 .3.2.65.78.54A11.25 11.25 0 0 0 12 .75Z" />
      </svg>
      <span>{count}</span>
      <svg className="github-stars-star" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
        <path d="m12 2 3.09 6.26L22 9.27l-5 4.87 1.18 6.88L12 17.77l-6.18 3.25L7 14.14 2 9.27l6.91-1.01Z" />
      </svg>
    </a>
  )
}
