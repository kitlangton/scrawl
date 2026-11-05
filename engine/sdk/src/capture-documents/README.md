# Detached capture documents (unpublished)

These module-local helpers prepare the new capture channel's data boundary.
They are not package exports, a negotiated wire contract, a native provider or
evidence of capture readiness. Capture-v1 and media/1 are unchanged.

The helpers verify exact detached UTF-8 bytes and their SHA256, envelope/payload
identity, clock mapping, collection references, stroke facts and publication
inventory inputs. Publication preflight requires the original detached bytes
and requested timestamps for each referenced derivative receipt. Decoder PTS is
independent of requested seek time, including start/end clamps.

These checks do not authenticate a producer, read filesystem contents, establish
native ownership or prove durability. Native publication still must validate the
complete existing ApparentBundle version-1 schema and all referenced/retained
media, enforce source/lease/publication authority, and perform no-replace durable
publication with crash recovery. Required role presence alone does not prove a
valid bundle. There is no new mandatory semantic companion file.

Context/source/invocation schemas are not frozen. The materials helper preserves
those opaque inputs without claiming to validate their native facts. Do not use
it as the complete capture contract or bypass a missing capability.

Record's committed processing baseline is PR715 (`495715bf`): its TypeScript
formatter consumes target/session/context, transcript, selected-frame, note/mark
and warning facts and returns `summaryMarkdown` only. It owns their translation
and semantic choices. No canonical manifest writer or authenticated publication
consumer is supplied here. A new capture-channel allocator must persist its
identity/plan binding in a separate private control namespace and leave the final
destination absent. The existing v1 reservation-directory allocator is incompatible.
