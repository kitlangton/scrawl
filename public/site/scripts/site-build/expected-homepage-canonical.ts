import { parse } from "parse5"

const htmlNamespace = "http://www.w3.org/1999/xhtml"

// Parse inert HTML, not strings that merely resemble tags in comments, scripts
// or templates. A homepage must declare exactly one active canonical, in head.
export function hasExpectedHomepageCanonical(html) {
  const links = []
  function visit(node) {
    if (node.namespaceURI === htmlNamespace && node.tagName === "link") {
      const rel = node.attrs.find(({ name }) => name === "rel")?.value ?? ""
      if (rel.toLowerCase().split(/[\t\n\f\r ]+/).includes("canonical")) links.push(node)
    }
    for (const child of node.childNodes ?? []) visit(child)
  }
  visit(parse(html))
  if (links.length !== 1 || links[0].parentNode?.tagName !== "head") return false
  const href = links[0].attrs.find(({ name }) => name === "href")?.value
  if (!href) return false
  try {
    // No base: relative URLs must not borrow the expected site's identity.
    // Serialized equality also rejects empty query/fragment delimiters, while
    // permitting URL-equivalent host case and the default HTTPS port.
    const url = new URL(href)
    return url.href === "https://app.tryapparent.com/"
      && url.username === "" && url.password === ""
  } catch {
    return false
  }
}
