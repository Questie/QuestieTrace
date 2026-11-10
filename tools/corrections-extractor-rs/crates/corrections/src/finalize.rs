//! Merges the folded observations into one value per entity and field, builds the records and
//! renders the output modules. Ports `finalize` of `extract/index.ts` and `finalizeLookups` of
//! `extract/lookups.ts`.

use crate::fold::Accumulator;
use crate::lua::{write_corrections, write_quest_xp, Header, Js, Record};
use crate::observers::quest;

/// Rendered output: file name -> module, and lookup directory -> locale -> module.
pub struct Bundle {
    pub modules: Vec<(&'static str, String)>,
    pub lookups: Vec<(&'static str, Vec<(String, String)>)>,
}

pub fn finalize(acc: &Accumulator, source_file_count: usize, generated_at: &str) -> Bundle {
    let header = Header {
        source_file_count,
        session_count: acc.session_count(),
        generated_at,
    };

    let required_level = acc.required_level.merge(quest::required_level::merge);
    let quests: Vec<(i64, Record)> = required_level
        .into_iter()
        .map(|(id, fact)| (id, vec![("requiredLevel".to_owned(), Js::Num(fact.value))]))
        .collect();

    Bundle {
        modules: vec![
            ("foreverNpcTraces.lua", write_corrections("ForeverNpcTraces", "npcKeys", &[], &header)),
            ("foreverQuestTraces.lua", write_corrections("ForeverQuestTraces", "questKeys", &quests, &header)),
            ("foreverItemTraces.lua", write_corrections("ForeverItemTraces", "itemKeys", &[], &header)),
            ("foreverObjectTraces.lua", write_corrections("ForeverObjectTraces", "objectKeys", &[], &header)),
            ("xpDB-traces.lua", write_quest_xp(&[], &header)),
        ],
        lookups: vec![
            ("lookupNpcs", Vec::new()),
            ("lookupQuests", Vec::new()),
            ("lookupItems", Vec::new()),
            ("lookupObjects", Vec::new()),
        ],
    }
}
