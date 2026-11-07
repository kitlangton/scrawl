use super::*;
fn input() -> (Value, Value, u64) {
    let n = 9_007_199_254_740_993;
    let source = json!({"id":"display:123","generation":"opaque generation","kind":"display","name":"DP-1","bounds":{"x":10,"y":20,"width":100,"height":50,"scale":1}});
    let rect = json!({"x":10,"y":20,"width":100,"height":50});
    let surface = json!({"surfaceId":"screen","transformId":"t1","nativeIncarnation":"native-1","target":"capture-rectangle","pointSpace":"desktop-physical","contentRectangle":rect,"plane":{"width":100,"height":50}});
    let req = json!({"ingressVersion":1,"ownerToken":"a".repeat(64),"captureId":"b".repeat(32),"source":{"id":source["id"],"generation":source["generation"]},"observation":{"host":{"processEpoch":"epoch","revision":0,"pid":1,"startTicks":"1"},"producerId":"c".repeat(32),"generation":1,"bootId":"boot","clock":"linux-clock-monotonic-observation","output":{"name":"DP-1","physical":rect,"compositor":source["bounds"]},"surfaces":[surface]}});
    (req, source, n)
}
fn batch(r: &Registration, seq: u64, events: Value) -> Value {
    let raw = serde_json::to_string(&events).unwrap();
    json!({"ingressVersion":1,"captureId":"b".repeat(32),"registrationId":r.registration_id,"producerToken":r.token,"sequence":seq,"eventsJson":raw,"batchSha256":super::super::sha(raw.as_bytes())})
}
fn begin(n: u64) -> Value {
    json!({"kind":"begin","gestureId":"d".repeat(32),"surfaceId":"screen","transformId":"t1","nativeIncarnation":"native-1","timeNs":n.to_string(),"x":20,"y":30,"rgb":[12,34,56]})
}
fn finish_req(r: &Registration, seq: Value, n: u64) -> Value {
    json!({"ingressVersion":1,"captureId":"b".repeat(32),"registrationId":r.registration_id,"producerToken":r.token,"finalSequence":seq,"seal":{"state":"sealed-and-drained","observedAtNs":n.to_string(),"nativeRevision":1}})
}
#[test]
fn cross_batch_history_replays_and_survives_serialized_journal_state() {
    let (req, source, n) = input();
    let mut r = Registration::new(&req, &source, n, "boot").unwrap();
    let first = batch(&r, 0, json!([begin(n)]));
    assert_eq!(
        r.submit(&first, n + 9_000_000, &source).unwrap()["outcome"],
        "accepted"
    );
    let bytes = serde_json::to_vec(&r).unwrap();
    r = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        r.submit(&first, n + 9_000_000, &source).unwrap()["outcome"],
        "replayed"
    );
    let points = batch(
        &r,
        1,
        json!([{"kind":"points","gestureId":"d".repeat(32),"points":[{"timeNs":(n+1_000_000).to_string(),"x":150,"y":30},{"timeNs":(n+2_000_000).to_string(),"x":20,"y":30}]}]),
    );
    r.submit(&points, n + 9_000_000, &source).unwrap();
    let end = batch(
        &r,
        2,
        json!([{"kind":"commit","gestureId":"d".repeat(32),"acceptedAtNs":(n+3_000_000).to_string()}]),
    );
    r.submit(&end, n + 9_000_000, &source).unwrap();
    let finish = finish_req(&r, json!(2), n + 10_000_000);
    assert_eq!(
        r.finish(&finish, n + 10_000_000).unwrap()["state"],
        "complete"
    );
    assert_eq!(
        r.finish(&finish, n + 11_000_000).unwrap()["state"],
        "complete"
    );
    let f = r.facts(&"b".repeat(32), &"e".repeat(32)).unwrap();
    assert_eq!(f["strokes"].as_array().unwrap().len(), 2);
    assert_eq!(f["strokes"][0]["rgb"], json!([12, 34, 56]));
    assert_eq!(f["strokes"][0]["points"][0]["timeMs"], 0);
    for stroke in f["strokes"].as_array().unwrap() {
        for p in stroke["points"].as_array().unwrap() {
            assert!(p["x"].as_u64().unwrap() <= 10000);
        }
    }
}
#[test]
fn viewport_is_independent_of_board_plane_and_undo_preserves_history() {
    let (mut req, source, n) = input();
    req["observation"]["surfaces"] = json!([{"surfaceId":"board","transformId":"t1","nativeIncarnation":"board-1","target":"whiteboard-canvas","pointSpace":"paper-logical","contentRectangle":{"x":200,"y":164,"width":720,"height":476},"plane":{"width":1600,"height":1100}}]);
    let mut r = Registration::new(&req, &source, n, "boot").unwrap();
    let mut b = begin(n);
    b["surfaceId"] = json!("board");
    b["nativeIncarnation"] = json!("board-1");
    b["x"] = json!(800);
    b["y"] = json!(550);
    let ev = json!([b,{"kind":"points","gestureId":"d".repeat(32),"points":[{"timeNs":(n+1_000_000).to_string(),"x":1600,"y":1100}]},{"kind":"commit","gestureId":"d".repeat(32),"acceptedAtNs":(n+2_000_000).to_string()},{"kind":"clear","surfaceId":"board","transformId":"t1","nativeIncarnation":"board-1","observedAtNs":(n+3_000_000).to_string(),"nativeRevision":1}]);
    let batch = batch(&r, 0, ev);
    r.submit(&batch, n + 4_000_000, &source).unwrap();
    let finish = finish_req(&r, json!(0), n + 4_000_000);
    r.finish(&finish, n + 4_000_000).unwrap();
    let f = r.facts(&"b".repeat(32), &"e".repeat(32)).unwrap();
    assert_eq!(f["surfaces"][0]["width"], 1600);
    assert_eq!(f["strokes"][0]["points"][0]["x"], 5000);
    assert_eq!(f["strokes"][0]["points"][0]["y"], 5000);
}
#[test]
fn only_positive_empty_completion_collects_and_foreign_auth_cannot_invalidate() {
    let (req, source, n) = input();
    let mut r = Registration::new(&req, &source, n, "boot").unwrap();
    assert!(r.facts("c", "m").is_none());
    let mut foreign = batch(&r, 0, json!([begin(n)]));
    foreign["producerToken"] = json!("f".repeat(64));
    assert_eq!(r.submit(&foreign, n + 1, &source), Err("permission_denied"));
    assert!(r.collecting());
    let end = finish_req(&r, Value::Null, n + 1);
    assert_eq!(r.finish(&end, n + 1).unwrap()["state"], "complete");
    assert!(
        r.facts("c", "m").unwrap()["strokes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let mut r = Registration::new(&req, &source, n, "boot").unwrap();
    let bad = finish_req(&r, json!(0), n + 1);
    assert_eq!(r.finish(&bad, n + 1).unwrap()["state"], "unavailable");
    assert!(r.facts("c", "m").is_none());
}
#[test]
fn malformed_clock_foreign_transform_future_and_gap_fail_closed() {
    let (req, source, n) = input();
    for variant in ["clock", "transform", "future", "gap", "overflow"] {
        let mut r = Registration::new(&req, &source, n, "boot").unwrap();
        let mut e = begin(n);
        match variant {
            "clock" => e["timeNs"] = json!("01"),
            "transform" => e["transformId"] = json!("foreign"),
            "future" => e["timeNs"] = json!((n + 100).to_string()),
            "overflow" => e["x"] = json!(1e100),
            _ => (),
        };
        let b = batch(&r, if variant == "gap" { 1 } else { 0 }, json!([e]));
        assert!(r.submit(&b, n + 1, &source).is_err(), "{variant}");
        assert!(!r.collecting());
        assert!(r.facts("c", "m").is_none());
    }
}
#[test]
fn fractional_screen_mapping_matches_the_exact_donor_cast_order() {
    for (origin, x, expected) in [(0.0, 1.6319998999999998, 9), (10.0, 10.2879999, 1)] {
        let s = Surface {
            value: json!({"target":"capture-rectangle"}),
            x: origin,
            y: 0.0,
            width: 1920,
            height: 1080,
        };
        let donor = ((((x - origin) / 1920.0) as f32) * 10_000.0_f32).round() as i32;
        assert_eq!(donor, expected);
        assert_eq!(point(&json!({"x":x,"y":0}), &s).unwrap().0, donor);
    }
}
#[test]
fn closing_requires_the_exact_registered_incarnation_and_viewport() {
    let (mut req, source, n) = input();
    let board = json!({"surfaceId":"board","transformId":"t1","nativeIncarnation":"board-1","target":"whiteboard-canvas","pointSpace":"paper-logical","contentRectangle":{"x":200,"y":164,"width":720,"height":476},"plane":{"width":1600,"height":1100}});
    req["observation"]["surfaces"] = json!([board]);
    for field in ["nativeIncarnation", "geometry"] {
        let mut r = Registration::new(&req, &source, n, "boot").unwrap();
        let mut wrong = board.clone();
        if field == "geometry" {
            wrong["contentRectangle"]["width"] = json!(721)
        } else {
            wrong["nativeIncarnation"] = json!("foreign")
        }
        let b = batch(
            &r,
            0,
            json!([{"kind":"surface-state","state":"closed","surface":wrong,"observedAtNs":n.to_string(),"nativeRevision":1}]),
        );
        assert!(r.submit(&b, n + 1, &source).is_err());
        assert!(r.current.contains_key("board"));
    }
}
#[test]
fn post_stop_lifecycle_exception_never_admits_new_points_or_commit() {
    let (req, source, n) = input();
    for kind in ["points", "commit"] {
        let mut r = Registration::new(&req, &source, n, "boot").unwrap();
        let first = batch(&r, 0, json!([begin(n)]));
        r.submit(&first, n + 1, &source).unwrap();
        let late = if kind == "points" {
            json!({"kind":"points","gestureId":"d".repeat(32),"points":[{"timeNs":(n+2).to_string(),"x":30,"y":30}]})
        } else {
            json!({"kind":"commit","gestureId":"d".repeat(32),"acceptedAtNs":(n+2).to_string()})
        };
        let b = batch(&r, 1, json!([late]));
        assert!(r.submit_at(&b, n + 1, n + 3, &source).is_err());
        assert!(!r.collecting());
    }
}
