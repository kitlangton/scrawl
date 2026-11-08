import { Demo } from "../../demo/core/VideoDemo"
import { PhotoStory } from "./PhotoStory"
import { FrogPond } from "./FrogPond"
import { GithubStars } from "./GithubStars"
import { Footer } from "../../footer/Footer"
import { FeatureGrid } from "../../features/FeatureGrid"

export function App() {
  return (
    <div className="pond-page">
      <header className="pond-nav">
        <a className="brand" href="#top" aria-label="apparent home">
          <img src="/apparent-wordmark.svg" width={149} height={35} decoding="async" alt="apparent" />
        </a>
        <GithubStars />
      </header>
      <main>
        <FrogPond />
        <section className="pond-demo" aria-label="See apparent in action">
          <h2>See what <span className="demo-heading-product"><img src="/apparent-wordmark.svg" alt="apparent" width={120} height={28} /> captures</span></h2>
          <div className="demo-section" id="demo" aria-label="Product demo">
            <Demo />
          </div>
        </section>
        <FeatureGrid />
        <PhotoStory />
      </main>
      <Footer />
    </div>
  )
}
