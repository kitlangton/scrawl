SDK 0.4.1 public media/2 client acceptance

Source: 9c453b18665782404238b79ecfc6d579eba969be (#839).
Actual main engine delivery run42221787032 succeeded. Contracts127905374766,
SDK owner127905523522 and runtime audit127906806466 succeeded. SDK lane reused
verified same-repository PR component13502794671 from test merge9489d64e149074a545b85a5be3802447534099a8;
that merge and landed main both have tree d79269c6e2bbf7a1edc146779676f6c649c8fdbf.
This is declared verified reuse, not a fresh main build.

Artifact13504585253: stream-engine-linux-x64-8d8a158a201d75c93b79845b3aa18927064cd2afdfb15aa1c384352a316b7534.
Component50352 bytes SHA256 2c158c86584d9e9deddc4126f0af0548401d2d4fabe11ce4c95d1c10a59b4057.
Exact single regular payload: apparent-engine-sdk-0.4.1.tgz,50050bytes,
SHA256 f0c4b9777033bcde0463e0442074e5af714cddaed1c45452802012e66d21c156.
Receipt binding,size/hash,package version/exports,main attestation and workflow/source
verified. gh attestation verify --repo tryapparent/apparent exited0.
Fresh packed runtime + strict declaration consumer with admitted Record831 passed.

API: @apparent/engine-sdk/media2 exports Media2Client,Media2JobInput,
preflightMedia2Dispatch,launchMedia2Worker. Explicit media protocol2;
original request retained across derive/inspect/cancel, unknown transport outcomes
stay unknown, no timeout-as-cancel. Same private runtime root as original capture2.
Capture/media1/default transport surfaces retain their prior defaults.
No native media2 service/artifact availability, derivative origin, Saved,
installed activation,Mac media2 or full P0 claim.
