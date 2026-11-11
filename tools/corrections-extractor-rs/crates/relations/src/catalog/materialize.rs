//! Runs `forever-quests.lua` inside a QuestieDB checkout and reads its JSON. QuestieDB's own loader
//! and correction registry do the layering; this module only finds an interpreter and turns the
//! raw rows into typed ones.

use super::build::QuestRow;
use crate::model::QuestId;
use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The exporter, fed to the interpreter on stdin (`lua - <arguments>`), so the binary carries it
/// and nothing has to be written to disk.
const SCRIPT: &str = include_str!("forever-quests.lua");

/// Output of `forever-quests.lua`; see the header comment there.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializedQuests {
    /// Apply order: base data, Static Corrections, Derived Passes, Dynamic Corrections.
    pub sources: Vec<String>,
    /// Quest id -> row after Static Corrections and Derived Passes.
    pub quests: BTreeMap<QuestId, QuestRow>,
    /// Quest id -> distinct rows the faction x class views read, for quests a Dynamic Correction
    /// touches. Every real character sees one of these, never the static row.
    pub dynamic_views: BTreeMap<QuestId, Vec<QuestRow>>,
    /// Quest id -> fields (Questie names) an authored forever*Fixes.lua provider replaces, adds to
    /// or removes from.
    pub authored_fields: BTreeMap<QuestId, Vec<String>>,
    /// Quests from the inherited Era baseline (data/Forever rows, Forever/legacy/ providers). The
    /// rest are Forever-new.
    pub inherited_ids: Vec<QuestId>,
}

impl MaterializedQuests {
    /// The rows real characters read for a quest: its dynamic views, or else its one static row.
    pub fn rows_of(&self, id: QuestId) -> &[QuestRow] {
        match self.dynamic_views.get(&id) {
            Some(views) => views,
            None => self.quests.get(&id).map(std::slice::from_ref).unwrap_or_default(),
        }
    }

    /// Every quest id with a static row or dynamic views, ascending.
    pub fn quest_ids(&self) -> impl Iterator<Item = QuestId> + '_ {
        let mut ids: Vec<QuestId> = self.quests.keys().chain(self.dynamic_views.keys()).copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MaterializeOptions<'a> {
    /// Run each Forever/traces/ provider from the same-named forever*Traces.lua in this directory
    /// and leave out those it has none for: QuestieDB as it loads with these modules imported,
    /// whatever the checkout holds. An empty directory leaves out every trace module.
    pub trace_modules_dir: Option<&'a Path>,
    /// Tests only: inline base rows and correction layers instead of QuestieDB's files.
    pub fixture: Option<&'a Path>,
}

/// QuestieDB's bundled Lua 5.1 interpreter (tools/lua-binary/, x64 only) when it fits this
/// machine, otherwise `lua5.1` from PATH.
pub fn lua_interpreter(questie_db_dir: &Path) -> PathBuf {
    let bundled = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux-x64/lua"),
        ("windows", "x86_64") => Some("lua.exe"),
        _ => None,
    };
    bundled
        .map(|name| questie_db_dir.join("tools/lua-binary").join(name))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("lua5.1"))
}

/// `fields`: Questie quest field names to export (src/meta/questMeta.lua keys).
pub fn materialize_forever_quests(questie_db_dir: &Path, fields: &[&str], options: MaterializeOptions) -> Result<MaterializedQuests> {
    // The script runs with the checkout as its working directory, so every path must be absolute.
    let questie_db_dir = &std::path::absolute(questie_db_dir)?;
    if !questie_db_dir.join("generator/flavor.lua").exists() {
        bail!(
            "{} is not a QuestieDB checkout (no generator/flavor.lua). Set QUESTIEDB_DIR.",
            questie_db_dir.display()
        );
    }
    let mut args: Vec<OsString> = vec!["-".into(), fields.join(",").into()];
    if let Some(dir) = options.trace_modules_dir {
        args.push("--trace-modules".into());
        args.push(std::path::absolute(dir)?.into_os_string());
    }
    if let Some(fixture) = options.fixture {
        args.push(std::path::absolute(fixture)?.into_os_string());
    }

    let lua = lua_interpreter(questie_db_dir);
    let mut child = Command::new(&lua)
        .args(&args)
        .current_dir(questie_db_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("starting {} (QuestieDB's catalog export needs Lua 5.1)", lua.display()))?;
    // Lua reads the whole chunk before running it, so this write never waits on stdout.
    let mut stdin = child.stdin.take().context("Lua's stdin")?;
    stdin.write_all(SCRIPT.as_bytes()).context("writing forever-quests.lua to Lua")?;
    drop(stdin);
    let output = child.wait_with_output().context("running forever-quests.lua")?;
    if !output.status.success() {
        bail!("forever-quests.lua failed ({}) in {}", output.status, questie_db_dir.display());
    }
    serde_json::from_slice(&output.stdout).context("reading forever-quests.lua's JSON")
}

/// Runs `forever-quests.lua` over inline correction layers through QuestieDB's real registry, so
/// the layering rules are checked as the catalog actually sees them. They need a QuestieDB
/// checkout (QUESTIEDB_DIR) and Lua 5.1, so they are ignored by default:
/// `QUESTIEDB_DIR=... cargo test -p relations -- --ignored`.
#[cfg(test)]
mod tests {
    use super::super::build::{build_catalog, build_ground_truth, exported_fields};
    use super::*;
    use crate::model::{RelationField, RelationSet, Starters};

    const FIXTURE: &str = r#"
return function(questKeys)
  return {
    base = {
      [10] = { [questKeys.name] = "Classic", [questKeys.startedBy] = {{1}}, [questKeys.preQuestSingle] = {1, 2},
               [questKeys.exclusiveTo] = {7}, [questKeys.nextQuestInChain] = 5, [questKeys.requiredRaces] = 4294967373 },
    },
    layers = {
      { name = "Forever/legacy/classicQuestFixes.lua:Load", rows = {
        [10] = { [questKeys.preQuestSingle] = {3}, [questKeys.exclusiveTo] = {} },
        [65593] = { [questKeys.name] = "Era event", [questKeys.nextQuestInChain] = 65594 },
      } },
      { name = "Forever/generated/foreverBaseQuest.lua:Load", rows = {
        [9700] = { [questKeys.name] = "New", [questKeys.startedBy] = {{100}} },
      } },
      { name = "Forever/traces/foreverQuestTraces.lua:Load", rows = {
        [9700] = { [questKeys.startedBy_add] = {{101}, nil, {5}} },
        [9701] = { [questKeys.name] = "Traced", [questKeys.startedBy_add] = {{200}} },
      } },
      { name = "Forever/foreverQuestFixes.lua:Load", rows = {
        [10] = { [questKeys.preQuestSingle_add] = {4}, [questKeys.preQuestSingle_remove] = {3} },
        [9700] = { [questKeys.preQuestSingle] = {10} },
      } },
      { name = "Forever/legacy/classicQuestFixes.lua:LoadFactionFixes", dynamic = true, rows = function()
        if UnitFactionGroup("player") == "Horde" then return { [10] = { [questKeys.nextQuestInChain] = 6 } } end
        return {}
      end },
    },
  }
end
"#;

    fn questie_db_dir() -> PathBuf {
        PathBuf::from(std::env::var_os("QUESTIEDB_DIR").expect("set QUESTIEDB_DIR to a QuestieDB checkout"))
    }

    /// A new directory under the system temp dir, so tests running side by side never share files.
    fn scratch_dir(name: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("relations-catalog-{}-{n}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn materialize_fixture(trace_modules_dir: Option<&Path>) -> MaterializedQuests {
        let fixture = scratch_dir("fixture").join("fixture.lua");
        std::fs::write(&fixture, FIXTURE).unwrap();
        let options = MaterializeOptions {
            trace_modules_dir,
            fixture: Some(&fixture),
        };
        materialize_forever_quests(&questie_db_dir(), &exported_fields(), options).unwrap()
    }

    fn relations(entries: &[(RelationField, &[QuestId])]) -> RelationSet {
        entries.iter().map(|&(field, targets)| (field, targets.to_vec())).collect()
    }

    #[test]
    #[ignore = "needs a QuestieDB checkout in QUESTIEDB_DIR and Lua 5.1"]
    fn applies_replace_add_remove_deletion_and_per_faction_dynamic_corrections_in_layer_order() {
        let truth = build_ground_truth(&materialize_fixture(None), "");
        use RelationField::*;
        assert_eq!(
            truth.tiers.inherited_classic,
            BTreeMap::from([
                // preQuestSingle {1,2} -> replaced by {3} -> authored removes 3, adds 4. exclusiveTo deleted by {}.
                // Horde's Dynamic Correction changes nextQuestInChain; Alliance keeps 5.
                (10, relations(&[(PreQuestSingle, &[4]), (NextQuestInChain, &[5, 6])])),
                // Created by a legacy fix, so inherited whatever its id.
                (65593, relations(&[(NextQuestInChain, &[65594])])),
            ])
        );
        // 9700 and 9701 are Forever-new; 9701 has no authored relation, so it is not ground truth.
        assert_eq!(truth.tiers.authored_forever, BTreeMap::from([(9700, relations(&[(PreQuestSingle, &[10])]))]));
    }

    #[test]
    #[ignore = "needs a QuestieDB checkout in QUESTIEDB_DIR and Lua 5.1"]
    fn keeps_race_bits_above_2_to_the_32_exact_and_adds_trace_starters_to_the_base_ones() {
        let quests = build_catalog(&materialize_fixture(None), "").quests;
        assert_eq!(quests[&10].required_races, Some(4294967373));
        assert_eq!(
            quests[&9700].starters,
            Starters {
                npcs: vec![100, 101],
                objects: vec![],
                items: vec![5]
            }
        );
        assert_eq!(
            (quests[&9701].name.as_deref(), quests[&9701].starters.npcs.as_slice()),
            (Some("Traced"), &[200][..])
        );
    }

    #[test]
    #[ignore = "needs a QuestieDB checkout in QUESTIEDB_DIR and Lua 5.1"]
    fn a_trace_modules_dir_without_a_module_leaves_that_module_out() {
        // So a dump never reads its own earlier output as baseline.
        let empty = scratch_dir("no-trace-modules");
        let quests = build_catalog(&materialize_fixture(Some(&empty)), "").quests;
        assert_eq!(
            quests[&9700].starters,
            Starters {
                npcs: vec![100],
                objects: vec![],
                items: vec![]
            }
        );
        assert!(!quests.contains_key(&9701));
    }

    #[test]
    #[ignore = "needs a QuestieDB checkout in QUESTIEDB_DIR and Lua 5.1"]
    fn runs_a_dumps_own_trace_modules_in_place_of_the_checkouts() {
        let dir = scratch_dir("trace-modules");
        let module = r#"
local ForeverQuestTraces = QuestieLoader:CreateModule("ForeverQuestTraces")
local QuestieDB = QuestieLoader:ImportModule("QuestieDB")
function ForeverQuestTraces:Load()
  return { [999990] = { [QuestieDB.questKeys.name] = "Dump-only quest" } }
end
"#;
        std::fs::write(dir.join("foreverQuestTraces.lua"), module).unwrap();
        let empty = scratch_dir("no-trace-modules");
        let options = |trace_modules_dir| MaterializeOptions {
            trace_modules_dir: Some(trace_modules_dir),
            fixture: None,
        };
        let beneath = materialize_forever_quests(&questie_db_dir(), &["name"], options(&empty)).unwrap();
        let mut replaced = materialize_forever_quests(&questie_db_dir(), &["name"], options(&dir)).unwrap();
        // Everything the checkout's own trace modules added is gone; only the dump's module adds anything.
        let added = replaced.quests.remove(&999990).expect("the dump's quest");
        assert_eq!(added.name.as_deref(), Some("Dump-only quest"));
        assert_eq!(replaced.quests, beneath.quests);
        let source = "src/corrections/Forever/traces/foreverQuestTraces.lua:Load (run from the dump's foreverQuestTraces.lua)";
        assert!(replaced.sources.iter().any(|s| s == source), "{:?}", replaced.sources);
    }
}
