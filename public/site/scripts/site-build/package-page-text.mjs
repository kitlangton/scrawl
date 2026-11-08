import { parse } from "parse5"

// Parser-only marker guard: body text minus inactive/explicitly hidden subtrees.
// This is not proof of CSS/layout visibility: external CSS and dynamic state
// are outside scope. Only literal inline display:none declarations are checked;
// no CSS cascade, escapes, variables or layout are evaluated. See clanker-deploy.md.
// This is text extraction, not an HTML sanitizer.
// The result may contain literal angle brackets and must never be rendered as HTML.
export function packagePageText(html) {
  const document = parse(html)
  const root = document.childNodes.find((node) => node.tagName === "html")
  const body = root?.childNodes.find((node) => node.tagName === "body")
  const text = []
  function visit(node) {
    if (["script", "style", "template", "noscript", "iframe"].includes(node.tagName)) return
    if (node.attrs?.some(({ name }) => name === "hidden")) return
    const style = node.attrs?.find(({ name }) => name === "style")?.value ?? ""
    if (style.split(";").some((declaration) => /^\s*display\s*:\s*none\s*(?:!\s*important\s*)?$/i.test(declaration))) return
    if (node.nodeName === "#text") text.push(node.value)
    for (const child of node.childNodes ?? []) visit(child)
  }
  if (body) visit(body)
  return text.join("")
}
