Connect producer prerequisite

connect-delivery.yml builds and verifies browser and Worker outputs on main push
or manual main dispatch, without native CI. It uploads separate immutable Actions
artifacts named connect-browser-<full SHA> and connect-worker-<full SHA>, each
containing manifest.json and dist/. No publisher consumes these artifacts yet;
web-delivery retains sole publication authority.

Run `node --experimental-strip-types connect/delivery/artifacts.ts seal DIR KIND`
then `validate DIR KIND` from a clean committed checkout. KIND is connect-browser
or connect-worker. The manifest records source identity, input and output hashes,
production environment and runtime versions. This is local integrity evidence,
not independent trusted GitHub-run admission; Public owns that integration.

Producer analytics covers shared configuration, production artifacts and synthetic
Connect consent behavior. Public owns composed analytics transport acceptance.
Five browser assets still come from Public. Existing
Python theme checks and native Connect client/consumer coupling remain; this
producer is not complete Connect language or delivery independence.
