//! Extracts Questie DB corrections and translations from QuestieTrace submissions. A port of
//! `tools/corrections-extractor`; see DESIGN.md.

pub mod accumulator;
pub mod aggregate;
pub mod finalize;
pub mod guid;
pub mod item_link;
pub mod js;
pub mod lua;
pub mod observation;
pub mod observe;
pub mod observers;
pub mod pipeline;
pub mod probes;
pub mod resources;
pub mod run;

#[cfg(test)]
pub(crate) mod test_support;
