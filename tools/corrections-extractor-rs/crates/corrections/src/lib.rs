//! Extracts Questie DB corrections and translations from QuestieTrace submissions. A port of
//! `tools/corrections-extractor`; see DESIGN.md.

pub mod aggregate;
pub mod guid;
pub mod item_link;
pub mod js;
pub mod observation;
pub mod observers;
pub mod probes;
pub mod resources;

#[cfg(test)]
pub(crate) mod test_support;
