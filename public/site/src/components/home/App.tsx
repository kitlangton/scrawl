import { Demo } from "../../demo/core/VideoDemo"
import { BeliefPond } from "./BeliefPond"
import { FrogPond } from "./FrogPond"
import { GithubStars } from "./GithubStars"
import { Footer } from "../../footer/Footer"
import { Team } from "./Team"
import { FeatureGrid } from "../../features/FeatureGrid"

export function App() {
  return (
    <div className="pond-page">
      <header className="pond-nav">
        <a className="brand" href="#top" aria-label="Apparent home">
          <img src="/apparent-wordmark.svg" width={149} height={35} decoding="async" alt="apparent" />
        </a>
        <a className="pond-team-link" href="#team">Team</a>
        <GithubStars />
      </header>
      <main>
        <FrogPond />
        <section className="pond-demo" aria-label="See Apparent in action">
          <h2>See what <span>apparent</span> captures</h2>
          <div className="demo-section" id="demo" aria-label="Product demo">
            <Demo />
          </div>
        </section>
        <FeatureGrid />
        <BeliefPond />
        <Team />
      </main>
      <Footer />
    </div>
  )
}
