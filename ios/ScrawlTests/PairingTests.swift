import Foundation
@testable import Scrawl
import Testing

struct PairingTests {
  func link(_ text: String) -> Pairing.Link? { Pairing.parse(URL(string: text)!) }

  @Test func readsTheLinkScrawlLinkPrints() {
    let parsed = link("scrawl://scrawl?url=http%3A%2F%2F100.64.0.12%3A47777%2F&url=http%3A%2F%2F192.168.1.20%3A47777&token=deadbeef&session=ses_abc")
    #expect(parsed == .pair(Pairing(
      urls: [URL(string: "http://100.64.0.12:47777")!, URL(string: "http://192.168.1.20:47777")!],
      token: "deadbeef",
      session: "ses_abc"
    )))
  }

  @Test func aLinkMadeForAScrawlKitAppPairsThisOneToo() {
    guard case let .pair(pairing) = link("updown://scrawl?url=http%3A%2F%2F10.0.0.2%3A47777&token=t") else {
      Issue.record("not a pairing")
      return
    }
    #expect(pairing.urls == [URL(string: "http://10.0.0.2:47777")!])
    #expect(pairing.session == nil)
    #expect(!pairing.isHosted)
    #expect(link("updown://tether?url=http%3A%2F%2F10.0.0.2%3A47777&token=t") != nil)
  }

  @Test func addressesAreHTTPOnceEachAndATokenIsNeeded() {
    #expect(link("scrawl://scrawl?url=ftp%3A%2F%2F10.0.0.2&token=t") == nil)
    #expect(link("scrawl://scrawl?url=http%3A%2F%2F10.0.0.2%3A47777") == nil)
    #expect(link("scrawl://scrawl?url=http%3A%2F%2F10.0.0.2%3A47777&token=%20") == nil)
    #expect(link("scrawl://elsewhere?url=http%3A%2F%2F10.0.0.2%3A47777&token=t") == nil)
    guard case let .pair(pairing) = link("scrawl://scrawl?url=http%3A%2F%2F10.0.0.2%3A47777%2F%2F&url=http%3A%2F%2F10.0.0.2%3A47777&url=https%3A%2F%2Fproblems.example.com%2Fapi%2Fscrawl&token=t&session=%20")
    else {
      Issue.record("not a pairing")
      return
    }
    #expect(pairing.urls.map(\.absoluteString) == ["http://10.0.0.2:47777", "https://problems.example.com/api/scrawl"])
    #expect(pairing.session == nil)
    #expect(pairing.isHosted)
  }

  @Test func resetLinksUnpair() {
    #expect(link("scrawl://scrawl?reset=1") == .reset)
    #expect(link("scrawl://scrawl?reset=true&token=t") == .reset)
  }

  @Test func aPastedTerminalLineHasTheLinkInIt() {
    let pasted = "  'scrawl://scrawl?url=http%3A%2F%2F10.0.0.2%3A47777&token=abc'\n"
    #expect(Pairing.parse(text: pasted) == .pair(Pairing(urls: [URL(string: "http://10.0.0.2:47777")!], token: "abc")))
    #expect(Pairing.parse(text: "hello there") == nil)
  }
}
