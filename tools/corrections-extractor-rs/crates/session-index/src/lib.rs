//! Lists every recorded session of a trace-data checkout once, and which submissions hold
//! nothing that is not kept from elsewhere.
//!
//! An export holds every saved session plus the one in progress, and older addon versions kept
//! reported sessions, so one session reaches trace-data in many submissions: repeated as it was,
//! or longer as play went on. Copies of a session share its capture-start clocks
//! ([`trace_data::Session::key`]); of each session one copy is kept and the others are
//! superseded (see [`index`] for which).
//!
//! Read-only: [`scan::scan_checkout`] reads the checkout, [`index::Index::build`] decides,
//! [`output::write_index`] writes the result elsewhere for cleanup scripts to act on.

pub mod index;
pub mod output;
pub mod scan;

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests {
    use crate::index::Index;
    use crate::output::write_index;
    use crate::scan::scan_checkout;
    use crate::test_support::{export_string, session, TempDir};
    use serde_json::json;

    #[test]
    fn indexes_a_checkout_read_through_the_real_decoder() {
        let checkout = TempDir::new("checkout");
        let submission = |key: &str, export_string: &str| {
            let path = checkout.0.join("submissions").join(key);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let json = json!({ "id": key, "contributor_id": "c0ffee", "nickname": "Thrall", "export_string": export_string });
            std::fs::write(path, json.to_string()).unwrap();
        };
        let (s, t) = (Some((1000.0, 2000.5)), Some((5.0, 6.25)));
        let mut stopped_s = session(s, 3);
        stopped_s["name"] = json!("Thrall's run");
        stopped_s["stoppedAt"] = json!(1010.0);
        stopped_s["functionsDelta"] = json!({ "X": [1] });
        // Older addon versions named a session when it started, so a name does not mean stopped.
        let mut named_s = session(s, 2);
        named_s["name"] = json!("Thrall's run");
        // `functions = {}` decodes to an empty array: no valid session.
        let invalid = json!({ "events": [{ "t": 0, "tp": 0, "e": "PLAYER_LOGIN", "a": { "n": 0 } }], "functions": {} });
        let cut_off = &export_string(&[session(s, 1)])[..40];
        submission("2026-09/2026-09-30_000000Z_x.json", &export_string(&[session(t, 1), invalid]));
        // s in progress, and t.
        submission("2026-10/2026-10-01_000000Z_a.json", &export_string(&[named_s, session(t, 1)]));
        // s stopped, with more events.
        submission("2026-10/2026-10-02_000000Z_b.json", &export_string(&[stopped_s]));
        submission("2026-10/2026-10-03_000000Z_c.json", "!QuestieTrace:1!abc$def!End:QuestieTrace:1!");
        // t again as it was, and a session without clocks, in the export after one cut off.
        submission(
            "2026-10/2026-10-04_000000Z_d.json",
            &format!("{cut_off}{}", export_string(&[session(t, 1), session(None, 1)])),
        );
        let output = TempDir::new("output");

        let index = Index::build(scan_checkout(&checkout.0, &|_| {}).unwrap());
        write_index(&index, &output.0).unwrap();

        let read = |name: &str| std::fs::read_to_string(output.0.join(name)).unwrap();
        // Every copy's function samples are `GetLocale = [{t = 0, tp = 0, v = "enUS"}]`: 16 bytes of
        // CBOR. The stopped copy's `functionsDelta` adds 5: a1 61 58 81 01.
        assert_eq!(
            read("sessions.jsonl"),
            concat!(
                r#"{"startedAt":1000.0,"startedAtPrecise":2000.5,"#,
                r#""kept":{"submission":"2026-10/2026-10-02_000000Z_b.json","export":0,"session":0,"events":3,"functionBytes":21,"stopped":true},"#,
                r#""superseded":[{"submission":"2026-10/2026-10-01_000000Z_a.json","export":0,"session":0,"events":2,"functionBytes":16,"stopped":false}]}"#,
                "\n",
                r#"{"startedAt":5.0,"startedAtPrecise":6.25,"#,
                r#""kept":{"submission":"2026-10/2026-10-04_000000Z_d.json","export":1,"session":0,"events":1,"functionBytes":16,"stopped":false},"#,
                r#""superseded":[{"submission":"2026-09/2026-09-30_000000Z_x.json","export":0,"session":0,"events":1,"functionBytes":16,"stopped":false},"#,
                r#"{"submission":"2026-10/2026-10-01_000000Z_a.json","export":0,"session":1,"events":1,"functionBytes":16,"stopped":false}]}"#,
                "\n",
                r#"{"startedAt":null,"startedAtPrecise":null,"#,
                r#""kept":{"submission":"2026-10/2026-10-04_000000Z_d.json","export":1,"session":1,"events":1,"functionBytes":16,"stopped":false},"#,
                r#""superseded":[]}"#,
                "\n",
            )
        );
        assert_eq!(
            read("submissions.jsonl"),
            concat!(
                r#"{"submission":"2026-09/2026-09-30_000000Z_x.json","status":"needed","copies":1,"keptCopies":0,"droppedExports":0,"invalidSessions":1}"#,
                "\n",
                r#"{"submission":"2026-10/2026-10-01_000000Z_a.json","status":"redundant","copies":2}"#,
                "\n",
                r#"{"submission":"2026-10/2026-10-02_000000Z_b.json","status":"needed","copies":1,"keptCopies":1,"droppedExports":0,"invalidSessions":0}"#,
                "\n",
                r#"{"submission":"2026-10/2026-10-03_000000Z_c.json","status":"unreadable","error":"export 1: payload is not valid print-encoded data (character \"$\")"}"#,
                "\n",
                r#"{"submission":"2026-10/2026-10-04_000000Z_d.json","status":"needed","copies":2,"keptCopies":2,"droppedExports":1,"invalidSessions":0}"#,
                "\n",
            )
        );
        assert_eq!(
            read("summary.json"),
            r#"{
  "submissions": 5,
  "unreadableSubmissions": 1,
  "emptySubmissions": 0,
  "redundantSubmissions": 1,
  "neededSubmissions": 3,
  "sessionCopies": 6,
  "uniqueSessions": 3,
  "supersededCopies": 3,
  "smallerCopies": 1,
  "sessionsWithoutKey": 1,
  "sessionsSharingClocks": 0,
  "invalidSessions": 1
}
"#
        );
    }
}
