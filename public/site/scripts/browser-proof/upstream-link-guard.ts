import { parse } from "parse5"

const publicSite = "https://app.tryapparent.com/"
const htmlNamespace = "http://www.w3.org/1999/xhtml"

function parsedURL(value, base) {
  try {
    return new URL(value, base)
  } catch {
    return null
  }
}

function documentBase(element) {
  const url = parsedURL(element?.attrs.find((attr) => attr.name === "href").value ?? publicSite, publicSite)
  return !url || ["data:", "javascript:", "vbscript:"].includes(url.protocol) ? new URL(publicSite) : url
}

function firstDocumentBase(node) {
  if (node.namespaceURI === htmlNamespace && node.tagName === "base" && node.attrs.some((attr) => attr.name === "href")) return node
  // Only document children: template content is a separate inert fragment.
  for (const child of node.childNodes ?? []) {
    const base = firstDocumentBase(child)
    if (base) return base
  }
}

function isUpstream(url) {
  if (!url || !["http:", "https:"].includes(url.protocol)) return false
  const hostname = url.hostname.toLowerCase().replace(/\.$/, "")
  if (hostname === "apparent.kitlangton.com") return true
  if (!["github.com", "www.github.com", "raw.githubusercontent.com", "codeload.github.com"].includes(hostname)) return false
  let parts
  try {
    parts = url.pathname.split("/").map((part) => decodeURIComponent(part).toLowerCase())
  } catch {
    return false
  }
  return parts[1] === "kitlangton" && ["tap", "homebrew-tap"].includes(parts[2]?.replace(/\.git$/, ""))
}

// The URL-bearing portion of HTML's shared declarative refresh steps. This
// does not run timers or scripts; delay-only refreshes reload the current page.
function refreshDestination(content) {
  // Chromium consumes repeated decimal points too (for example 0..5).
  // A run of dots without any digit still does not start a refresh.
  const delay = /^[\t\n\f\r ]*[0-9.]+/.exec(content)
  if (!delay || !/[0-9]/.test(delay[0])) return null
  let rest = content.slice(delay[0].length)
  if (!rest || !/^[;,\t\n\f\r ]/.test(rest)) return null
  rest = rest.replace(/^[\t\n\f\r ]*/, "")
  if (/^[;,]/.test(rest)) rest = rest.slice(1)
  rest = rest.replace(/^[\t\n\f\r ]*/, "")
  // URL= is optional. An incomplete prefix remains part of the URL, not a
  // reason to search later in the string for something that looks like a URL.
  rest = rest.replace(/^url[\t\n\f\r ]*=[\t\n\f\r ]*/i, "")
  if (rest[0] === "'" || rest[0] === '"') {
    const quote = rest[0]
    rest = rest.slice(1)
    const end = rest.indexOf(quote)
    if (end !== -1) rest = rest.slice(0, end)
  }
  return rest || null
}

// Parse HTML attributes (including character references) before interpreting URLs.
// Only destinations count: prose, comments, query strings and lookalike hosts do not.
export function hasUpstreamLinks(html) {
  const document = parse(html, { sourceCodeLocationInfo: true })
  const elements = []
  const inertElements = new Set()
  let baseElement
  let visibleText = ""
  function visit(node, inert = false) {
    if (["script", "style"].includes(node.tagName)) return
    if (node.attrs) {
      elements.push(node)
      if (inert) inertElements.add(node)
    }
    if (!inert && !baseElement && node.namespaceURI === htmlNamespace && node.tagName === "base" && node.attrs.some((attr) => attr.name === "href")) baseElement = node
    if (node.nodeName === "#text") visibleText += node.value
    for (const child of node.childNodes ?? []) visit(child, inert)
    // Template links can become active later, so inspect their inert contents too.
    // Their base elements must never change the active document's URL base.
    if (node.content) visit(node.content, true)
    if (node.tagName) visibleText += " "
  }
  visit(document)
  const base = documentBase(baseElement)
  for (const node of elements) {
    if (node.namespaceURI === htmlNamespace && node.tagName === "meta" && node.attrs.some((attr) => attr.name === "http-equiv" && attr.value.toLowerCase() === "refresh")) {
      const content = node.attrs.find((attr) => attr.name === "content")?.value ?? ""
      const destination = refreshDestination(content)
      // Inert template refreshes are checked conservatively against the final
      // base, as they have no active parser-processing point yet.
      if (destination !== null) {
        // Parse just the document available when this tag is processed. Let
        // HTML tree repair select its first base; source order is insufficient
        // under table foster parenting, and later bases must not affect it.
        const refreshBase = inertElements.has(node) ? base : documentBase(firstDocumentBase(
          parse(html.slice(0, node.sourceCodeLocation.startTag.endOffset)),
        ))
        if (isUpstream(parsedURL(destination, refreshBase))) return true
      }
    }
    for (const attr of node.attrs) {
      const metaURL = node.tagName === "meta" && attr.name === "content" && node.attrs.some((attribute) =>
        ["property", "name"].includes(attribute.name) && ["og:url", "og:image", "twitter:url", "twitter:image"].includes(attribute.value.toLowerCase()))
      if (["href", "src", "action", "formaction", "poster"].includes(attr.name) || metaURL) {
        if (isUpstream(parsedURL(attr.value, base))) return true
      }
    }
  }
  // The old guard also prevented upstream Homebrew install instructions. These
  // are formula tokens, not URLs; keep that protection with token boundaries.
  return /(?:^|[\s'"`;|&()<>])kitlangton\/tap(?:\/[a-z0-9@+_.-]+)?(?=$|[\s'"`;|&()<>])/i.test(visibleText)
}
