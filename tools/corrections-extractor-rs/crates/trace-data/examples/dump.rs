//! Prints each submission given on the command line as one JSON line, exactly as
//! `JSON.stringify` prints the TypeScript decoder's result, so the two decoders can be diffed:
//! `{"traces":[...],"dropped":[...],"error":"..."}`.

use trace_data::submission::Submission;
use trace_data::value::to_json;
use trace_data::Inflater;

fn json_string(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

fn main() {
    let mut inflater = Inflater::new();
    for path in std::env::args().skip(1) {
        let submission = Submission::read(path.as_ref(), &mut inflater);
        let decoded = submission.decode();
        let traces: Vec<String> = decoded.traces.iter().map(|trace| to_json(trace.root())).collect();
        let dropped: Vec<String> = decoded.dropped_exports.iter().map(|m| json_string(m)).collect();
        let error = decoded.error.as_deref().map_or("null".to_owned(), json_string);
        println!("{{\"traces\":[{}],\"dropped\":[{}],\"error\":{error}}}", traces.join(","), dropped.join(","));
    }
}
