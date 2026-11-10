//! Reads QuestieTrace submissions: listing and sampling a trace-data checkout, decoding the
//! exports of a submission, and a lazy view of each recorded session.
//!
//! ```no_run
//! # use trace_data::{source::list_submissions, submission::Submission};
//! let mut inflater = trace_data::Inflater::new();
//! for file in list_submissions("trace-data".as_ref())? {
//!     let submission = Submission::read(&file.path, &mut inflater);
//!     let decoded = submission.decode();
//!     for trace in &decoded.traces {
//!         for session in trace.sessions() {
//!             let _ = session.key();
//!         }
//!     }
//! }
//! # Ok::<(), std::io::Error>(())
//! ```

pub mod cbor;
pub mod export;
pub mod session;
pub mod source;
pub mod stream;
pub mod submission;
pub mod value;

pub use export::Inflater;
pub use session::{Event, Session, SessionKey};
pub use value::{format_number, JsKey, KeyRef, Object, Value};
