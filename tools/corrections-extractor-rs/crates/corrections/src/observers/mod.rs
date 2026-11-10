//! Field observers: one module per field and entity, each with its value type, observer and
//! (for fields with a custom merge) merge, next to their tests. See DESIGN.md.

pub mod guid_encounters;
pub mod item;
pub mod npc;
pub mod object;
pub mod quest;
pub mod spawn;
