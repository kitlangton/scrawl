import { createElement } from "react"
import { beliefMarkup } from "../../belief/markup"
import "../../belief/behavior"
import "../../belief/belief.css"

export function BeliefPond() {
  return <section className="pond-purpose">{createElement("apparent-belief", {
    className: "belief-pond",
    dangerouslySetInnerHTML: { __html: beliefMarkup },
  })}</section>
}
